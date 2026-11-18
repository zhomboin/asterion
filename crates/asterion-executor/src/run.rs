use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::{ExecError, confine_process_group, leader_exited};

const MAX_OUTPUT: usize = 24 * 1024;
const STREAM_OUTPUT: usize = MAX_OUTPUT / 2;
const PROGRESS_OUTPUT: usize = 16 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_REAP_LIMIT: Duration = Duration::from_millis(250);

#[derive(Serialize, Deserialize)]
pub(crate) struct ResultData {
    pub(crate) exit_code: Option<i32>,
    pub(crate) output: String,
    pub(crate) truncated: bool,
    pub(crate) duration_ms: u64,
}

pub struct RunProgress {
    pub output: String,
    pub truncated: bool,
}

pub(crate) type ProgressCallback<'a> = &'a (dyn Fn(RunProgress) + Send + Sync);

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

impl ResultData {
    pub(crate) fn empty(elapsed: Instant) -> Self {
        Self {
            exit_code: Some(0),
            output: String::new(),
            truncated: false,
            duration_ms: millis(elapsed),
        }
    }
}

struct ProcessGroup {
    leader: i32,
    armed: bool,
}

impl ProcessGroup {
    fn clean(&mut self) {
        if self.armed {
            // The unreaped group leader reserves this process group ID.
            unsafe { libc::kill(-self.leader, libc::SIGKILL) };
            self.armed = false;
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.clean();
    }
}

#[derive(Default)]
struct LiveOutput {
    streams: [Capture; 2],
    last_update: Option<Instant>,
}

struct Reporter<'a> {
    callback: ProgressCallback<'a>,
    output: Mutex<LiveOutput>,
}

impl Reporter<'_> {
    fn update(&self, channel: usize, bytes: &[u8], truncated: bool) {
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let Ok(mut output) = self.output.lock() else {
            return;
        };
        let stream = &mut output.streams[channel];
        if stream.bytes.len() == complete && stream.truncated == truncated {
            return;
        }
        stream
            .bytes
            .extend_from_slice(&bytes[stream.bytes.len()..complete]);
        stream.truncated = truncated;
        let now = Instant::now();
        if output
            .last_update
            .is_some_and(|previous| now.duration_since(previous) < PROGRESS_INTERVAL)
        {
            return;
        }
        let progress = render([&output.streams[0], &output.streams[1]], PROGRESS_OUTPUT);
        output.last_update = Some(now);
        drop(output);
        (self.callback)(progress);
    }
}

pub(crate) async fn command(
    program: &Path,
    args: &[String],
    deadline: Instant,
) -> Result<ResultData, ExecError> {
    command_with_progress(program, args, deadline, None).await
}

pub(crate) async fn command_with_progress(
    program: &Path,
    args: &[String],
    deadline: Instant,
    progress: Option<ProgressCallback<'_>>,
) -> Result<ResultData, ExecError> {
    if deadline <= Instant::now() {
        return Err(ExecError::TIMEOUT);
    }
    let started = Instant::now();
    let reporter = progress.map(|callback| Reporter {
        callback,
        output: Mutex::default(),
    });
    let mut command = Command::new(program);
    // The configured program and its direct descendants keep the task's process group.
    unsafe { command.pre_exec(confine_process_group) };
    let mut child = command
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(ExecError::io)?;

    let leader = child
        .id()
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(ExecError::FAILED)?;
    let mut group = ProcessGroup {
        leader,
        armed: true,
    };
    let stdout = child.stdout.take().ok_or(ExecError::FAILED)?;
    let stderr = child.stderr.take().ok_or(ExecError::FAILED)?;
    let result = tokio::time::timeout_at(deadline.into(), async {
        tokio::try_join!(
            wait_group(&mut child, &mut group),
            capture(stdout, 0, reporter.as_ref()),
            capture(stderr, 1, reporter.as_ref())
        )
    })
    .await;

    let (status, stdout, stderr) = match result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            drop(group);
            let _ = tokio::time::timeout(PROCESS_REAP_LIMIT, child.wait()).await;
            return Err(ExecError::io(error));
        }
        Err(_) => {
            drop(group);
            let _ = tokio::time::timeout(PROCESS_REAP_LIMIT, child.wait()).await;
            return Err(ExecError::TIMEOUT);
        }
    };
    Ok(result_data(status, stdout, stderr, started))
}

async fn wait_group(child: &mut Child, group: &mut ProcessGroup) -> io::Result<ExitStatus> {
    loop {
        match leader_exited(group.leader) {
            Ok(true) => {
                group.clean();
                return child.wait().await;
            }
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    group.armed = false;
                }
                return Err(error);
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn capture(
    mut input: impl AsyncRead + Unpin,
    channel: usize,
    reporter: Option<&Reporter<'_>>,
) -> io::Result<Capture> {
    let mut capture = Capture {
        bytes: Vec::with_capacity(4096),
        truncated: false,
    };
    let mut buffer = [0_u8; 4096];
    loop {
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let captures = (STREAM_OUTPUT - capture.bytes.len()).min(count);
        capture.bytes.extend_from_slice(&buffer[..captures]);
        let changed = captures > 0 || !capture.truncated;
        capture.truncated |= captures < count;
        if changed && let Some(reporter) = reporter {
            reporter.update(channel, &capture.bytes, capture.truncated);
        }
    }
    Ok(capture)
}

fn render(streams: [&Capture; 2], limit: usize) -> RunProgress {
    let mut output = String::new();
    let mut truncated = streams.iter().any(|stream| stream.truncated);
    for stream in streams {
        let text = String::from_utf8_lossy(&stream.bytes);
        for line in text.split_inclusive('\n') {
            if stream.truncated && !line.ends_with('\n') {
                break;
            }
            let clean: String = line
                .trim_end_matches('\n')
                .chars()
                .filter(|ch| !ch.is_control() || *ch == '\t')
                .collect();
            let redacted = asterion_collector::redact_output_line(&clean);
            if output.len() + redacted.len() + 1 > limit {
                truncated = true;
                break;
            }
            output.push_str(redacted);
            output.push('\n');
        }
    }
    RunProgress { output, truncated }
}

fn result_data(status: ExitStatus, stdout: Capture, stderr: Capture, start: Instant) -> ResultData {
    let result = render([&stdout, &stderr], MAX_OUTPUT);
    ResultData {
        exit_code: status
            .code()
            .or_else(|| status.signal().map(|value| 128 + value)),
        output: result.output,
        truncated: result.truncated,
        duration_ms: millis(start),
    }
}

fn millis(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A concurrent fork can inherit a writable executable descriptor until
    // exec and make the script launch fail with ETXTBSY.
    static COMMAND_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn captures_redacted_output_and_bounds_time() {
        let _fixture_guard = COMMAND_FIXTURES.lock().await;
        let progress = Mutex::new(Vec::new());
        let emit = |value: RunProgress| progress.lock().unwrap().push(value.output);
        let result = command_with_progress(
            Path::new("/usr/bin/printf"),
            &["app\nAuthorization: Bearer example\npartial".into()],
            Instant::now() + Duration::from_secs(3),
            Some(&emit),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.starts_with("app\n"));
        assert!(!result.output.contains("example"));
        let progress = progress.into_inner().unwrap();
        assert!(!progress.is_empty());
        assert!(
            progress
                .iter()
                .all(|text| !text.contains("example") && !text.contains("partial"))
        );

        let result = command(
            Path::new("/usr/bin/sleep"),
            &["30".into()],
            Instant::now() + Duration::from_millis(150),
        )
        .await;
        assert_eq!(result.err().unwrap().code, "timeout");
    }

    #[tokio::test]
    async fn successful_command_cleans_its_remaining_process_group() {
        let _fixture_guard = COMMAND_FIXTURES.lock().await;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
            .join(".local");
        std::fs::create_dir_all(&base).unwrap();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = base.join(format!("command-fixture-{}-{suffix}", std::process::id()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Cleanup(file);
        let mut script = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&fixture.0)
            .unwrap();
        use std::io::Write;
        script
            .write_all(b"#!/bin/sh\n/usr/bin/sleep 1 >/dev/null 2>&1 &\nprintf '%s\n' \"$!\"\n")
            .unwrap();
        script
            .set_permissions(std::fs::Permissions::from_mode(0o700))
            .unwrap();
        drop(script);
        let result = command(&fixture.0, &[], Instant::now() + Duration::from_secs(4))
            .await
            .unwrap();
        let pid = result.output.trim().parse::<u32>().unwrap();
        assert_eq!(result.exit_code, Some(0));
        let running = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|contents| {
                    contents
                        .rsplit_once(") ")
                        .and_then(|(_, fields)| fields.chars().next())
                })
                .is_some_and(|state| !matches!(state, 'Z' | 'X'))
        };
        for _ in 0..20 {
            if !running() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        panic!("fixture process unexpectedly survived its group leader");
    }
}
