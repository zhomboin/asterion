use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asterion_executor::{confine_process_group, leader_exited};
use asterion_protocol::{AgentReply, TaskRequest, is_node_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{Interest, unix::AsyncFd};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until, timeout};

const IDLE: Duration = Duration::from_secs(120);
const LIFETIME: Duration = Duration::from_secs(900);
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_READ: usize = 8192;
const MAX_INPUT: usize = 4096;

pub struct Terminals {
    enabled: bool,
    active: Option<Session>,
}

impl Terminals {
    pub fn new() -> Self {
        Self {
            enabled: std::env::var("ASTERION_TERMINAL_ENABLED").as_deref() == Ok("1")
                && unsafe { libc::geteuid() } != 0,
            active: None,
        }
    }

    pub fn handle(&mut self, request: TaskRequest) -> AgentReply {
        if request.kind == "terminal_capability" {
            return success(json!({ "enabled": self.enabled }));
        }
        if !self.enabled {
            return AgentReply::failure("terminal_disabled", "此节点未开启终端");
        }
        match request.kind.as_str() {
            "terminal_open" => self.open(request.args),
            "terminal_poll" => self.poll(request.args),
            "terminal_input" => self.input(request.args),
            "terminal_resize" => self.resize(request.args),
            "terminal_close" => self.close(request.args),
            _ => AgentReply::failure("invalid_request", "无效终端请求"),
        }
    }

    fn open(&mut self, args: Value) -> AgentReply {
        let Ok(args) = serde_json::from_value::<Open>(args) else {
            return invalid();
        };
        if !is_node_id(&args.id) || !valid_size(args.cols, args.rows) {
            return invalid();
        }
        if self
            .active
            .as_ref()
            .is_some_and(|session| !session.task.is_finished())
        {
            return AgentReply::failure("busy", "节点已有终端会话");
        }
        match start(args) {
            Ok(session) => {
                self.active = Some(session);
                success(json!({ "opened": true }))
            }
            Err(_) => AgentReply::failure("terminal_unavailable", "无法启动终端"),
        }
    }

    fn poll(&self, args: Value) -> AgentReply {
        let Ok(args) = serde_json::from_value::<Poll>(args) else {
            return invalid();
        };
        let Some(session) = self.session(&args.id) else {
            return missing();
        };
        let output = session
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let newest = output.first + output.bytes.len() as u64;
        if args.offset > newest {
            return invalid();
        }
        let start = args.offset.max(output.first);
        let skip = (start - output.first) as usize;
        let chunk: Vec<_> = output
            .bytes
            .iter()
            .skip(skip)
            .take(MAX_READ)
            .copied()
            .collect();
        success(json!({
            "offset": start + chunk.len() as u64,
            "data": STANDARD.encode(&chunk),
            "closed": session.task.is_finished() && start + chunk.len() as u64 == newest,
            "truncated": args.offset < output.first
        }))
    }

    fn input(&self, args: Value) -> AgentReply {
        let Ok(args) = serde_json::from_value::<Input>(args) else {
            return invalid();
        };
        if args.data.len() > MAX_INPUT.div_ceil(3) * 4 {
            return invalid();
        }
        let Ok(input) = STANDARD.decode(args.data) else {
            return invalid();
        };
        if input.is_empty() || input.len() > MAX_INPUT {
            return invalid();
        }
        self.send(&args.id, Control::Input(input))
    }

    fn resize(&self, args: Value) -> AgentReply {
        let Ok(args) = serde_json::from_value::<Open>(args) else {
            return invalid();
        };
        if !valid_size(args.cols, args.rows) {
            return invalid();
        }
        self.send(&args.id, Control::Resize(args.cols, args.rows))
    }

    fn close(&mut self, args: Value) -> AgentReply {
        let Ok(args) = serde_json::from_value::<Id>(args) else {
            return invalid();
        };
        if self.session(&args.id).is_none() {
            return missing();
        }
        self.active = None;
        success(Value::Null)
    }

    fn session(&self, id: &str) -> Option<&Session> {
        self.active.as_ref().filter(|session| session.id == id)
    }

    fn send(&self, id: &str, control: Control) -> AgentReply {
        let Some(session) = self.session(id) else {
            return missing();
        };
        match session.control.try_send(control) {
            Ok(()) => success(Value::Null),
            Err(mpsc::error::TrySendError::Closed(_)) => missing(),
            Err(mpsc::error::TrySendError::Full(_)) => AgentReply::failure("busy", "终端输入繁忙"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    id: String,
    cols: u16,
    rows: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Poll {
    id: String,
    offset: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: String,
    data: String,
}

struct Session {
    id: String,
    control: mpsc::Sender<Control>,
    output: Arc<Mutex<Output>>,
    task: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Control {
    Input(Vec<u8>),
    Resize(u16, u16),
}

#[derive(Default)]
struct Output {
    first: u64,
    bytes: VecDeque<u8>,
}

impl Output {
    fn push(&mut self, bytes: &[u8]) {
        let discarded = (self.bytes.len() + bytes.len()).saturating_sub(MAX_OUTPUT);
        self.bytes.drain(..discarded);
        self.first += discarded as u64;
        self.bytes.extend(bytes);
    }
}

struct Shell {
    child: Child,
    group: Option<i32>,
}

impl Shell {
    fn stop_group(&mut self) {
        if let Some(group) = self.group.take() {
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
    }

    fn reap_leader(&mut self) -> io::Result<bool> {
        let Some(group) = self.group else {
            return Ok(false);
        };
        match leader_exited(group) {
            Ok(true) => {
                self.stop_group();
                let _ = self.child.try_wait()?;
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => {
                // A reaped leader no longer reserves this numeric group ID.
                self.group = None;
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.stop_group();
        let _ = self.child.start_kill();
    }
}

fn start(args: Open) -> io::Result<Session> {
    let (master, slave) = open_pty(args.cols, args.rows)?;
    let parent = unsafe { libc::getpid() };
    let mut command = Command::new("/bin/sh");
    command
        .args(["-i", "+m"])
        .current_dir("/")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave))
        .kill_on_drop(true);
    // 调用仅使用已打开的标准输入，在 exec 前将其设为子进程的控制终端。
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ECHILD));
            }
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            confine_process_group()
        });
    }
    let child = command.spawn()?;
    let group = child
        .id()
        .ok_or_else(|| io::Error::other("missing child PID"))? as i32;
    let shell = Shell {
        child,
        group: Some(group),
    };
    let (tx, rx) = mpsc::channel(8);
    let output = Arc::new(Mutex::new(Output::default()));
    let session_output = Arc::clone(&output);
    let task = tokio::spawn(async move {
        let _ = drive(master, shell, rx, session_output).await;
    });
    Ok(Session {
        id: args.id,
        control: tx,
        output,
        task,
    })
}

async fn drive(
    master: AsyncFd<File>,
    mut shell: Shell,
    mut control: mpsc::Receiver<Control>,
    output: Arc<Mutex<Output>>,
) -> io::Result<()> {
    let mut absolute = Instant::now() + LIFETIME;
    let mut idle = Instant::now() + IDLE;
    let mut check_exit = interval(Duration::from_millis(100));
    check_exit.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut buffer = [0; MAX_READ];
    loop {
        tokio::select! {
            _ = sleep_until(absolute.min(idle)) => break,
            _ = check_exit.tick(), if shell.group.is_some() => {
                if shell.reap_leader()? {
                    // Leave a brief window to retain terminal output queued before shell exit.
                    absolute = absolute.min(Instant::now() + Duration::from_secs(1));
                }
            }
            read = master.async_io(Interest::READABLE, |mut file| file.read(&mut buffer)) => {
                match read {
                    Ok(0) => break,
                    Ok(count) => output.lock().unwrap_or_else(|error| error.into_inner()).push(&buffer[..count]),
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(error) => return Err(error),
                }
            }
            event = control.recv(), if shell.group.is_some() => match event {
                Some(Control::Input(data)) => {
                    idle = Instant::now() + IDLE;
                    timeout(Duration::from_secs(2), write(&master, &data)).await??;
                }
                Some(Control::Resize(cols, rows)) => {
                    let size = window(cols, rows);
                    if unsafe { libc::ioctl(master.get_ref().as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                None => break,
            }
        }
    }
    Ok(())
}

async fn write(master: &AsyncFd<File>, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let count = master
            .async_io(Interest::WRITABLE, |mut file| file.write(data))
            .await?;
        if count == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "terminal closed"));
        }
        data = &data[count..];
    }
    Ok(())
}

fn open_pty(cols: u16, rows: u16) -> io::Result<(AsyncFd<File>, File)> {
    let (mut master, mut slave) = (-1, -1);
    let size = window(cols, rows);
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for file in [&master, &slave] {
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let fd = master.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // File 独占描述符，注册后由 AsyncFd 持有且不会被重新分配。
    let master = unsafe { AsyncFd::register(master) }?;
    Ok((master, slave))
}

fn window(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn valid_size(cols: u16, rows: u16) -> bool {
    (20..=400).contains(&cols) && (5..=200).contains(&rows)
}

fn success(value: Value) -> AgentReply {
    AgentReply::Success { value }
}
fn invalid() -> AgentReply {
    AgentReply::failure("invalid_request", "无效终端参数")
}
fn missing() -> AgentReply {
    AgentReply::failure("terminal_closed", "终端会话已结束")
}

#[cfg(all(
    test,
    target_os = "linux",
    target_pointer_width = "64",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[tokio::test]
    async fn terminal_closes_when_leader_exits_with_background_job() {
        let session = start(Open {
            id: "a".repeat(32),
            cols: 80,
            rows: 24,
        })
        .unwrap();
        session
            .control
            .send(Control::Input(
                b"/usr/bin/sleep 20 & printf 'ready-%s\n' ok; exit\n".to_vec(),
            ))
            .await
            .unwrap();
        timeout(Duration::from_secs(4), async {
            while !session.task.is_finished() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("terminal and its background job must close after the shell exits");
        let output = session.output.lock().unwrap();
        let bytes: Vec<_> = output.bytes.iter().copied().collect();
        assert!(String::from_utf8_lossy(&bytes).contains("ready-ok"));
    }

    #[test]
    fn confined_child_cannot_leave_its_process_group() {
        let mut fixture = std::process::Command::new("/bin/true");
        unsafe {
            fixture.pre_exec(|| {
                confine_process_group()?;
                if libc::setpgid(0, 0) != -1
                    || io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
                    || libc::setsid() != -1
                    || io::Error::last_os_error().raw_os_error() != Some(libc::EPERM)
                {
                    return Err(io::Error::from_raw_os_error(libc::EPROTO));
                }
                Ok(())
            });
        }
        assert!(fixture.status().unwrap().success());
    }
}
