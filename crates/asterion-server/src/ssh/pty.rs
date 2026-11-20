use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::config::Target;
use asterion_protocol::{TaskReply, TaskRequest, is_node_id};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{Interest, unix::AsyncFd};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval, sleep_until, timeout};

const IDLE: Duration = Duration::from_secs(120);
const LIFETIME: Duration = Duration::from_secs(900);
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_READ: usize = 8192;
const MAX_INPUT: usize = 4096;

pub struct Terminals {
    active: Option<Session>,
}

impl Terminals {
    pub fn new() -> Self {
        Self { active: None }
    }
    pub async fn handle(
        &mut self,
        request: TaskRequest,
        target: &Target,
        container: Option<&str>,
    ) -> TaskReply {
        match request.kind.as_str() {
            "terminal_open" | "terminal_container_open" => {
                self.open(request.args, target, container).await
            }
            "terminal_poll" => self.poll(request.args),
            "terminal_input" => self.input(request.args),
            "terminal_resize" => self.resize(request.args),
            "terminal_close" => self.close(request.args),
            _ => TaskReply::failure("invalid_request", "无效终端请求"),
        }
    }

    async fn open(&mut self, args: Value, target: &Target, container: Option<&str>) -> TaskReply {
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
            return TaskReply::failure("busy", "节点已有终端会话");
        }
        match start(args, target, container).await {
            Ok(session) => {
                self.active = Some(session);
                success(json!({ "opened": true }))
            }
            Err(_) => TaskReply::failure("terminal_unavailable", "无法启动终端"),
        }
    }

    fn poll(&self, args: Value) -> TaskReply {
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

    fn input(&self, args: Value) -> TaskReply {
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

    fn resize(&self, args: Value) -> TaskReply {
        let Ok(args) = serde_json::from_value::<Open>(args) else {
            return invalid();
        };
        if !valid_size(args.cols, args.rows) {
            return invalid();
        }
        self.send(&args.id, Control::Resize(args.cols, args.rows))
    }

    fn close(&mut self, args: Value) -> TaskReply {
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

    fn send(&self, id: &str, control: Control) -> TaskReply {
        let Some(session) = self.session(id) else {
            return missing();
        };
        match session.control.try_send(control) {
            Ok(()) => success(Value::Null),
            Err(mpsc::error::TrySendError::Closed(_)) => missing(),
            Err(mpsc::error::TrySendError::Full(_)) => TaskReply::failure("busy", "终端输入繁忙"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
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

async fn start(args: Open, target: &Target, container: Option<&str>) -> io::Result<Session> {
    let (master, slave) = open_pty(args.cols, args.rows)?;
    let parent = unsafe { libc::getpid() };
    let mut command = target.command(true);
    const SHELL: &str = r#"/usr/bin/timeout -k 2 900 /bin/sh -c 'printf "\036ASTERION_READY\037"; exec /bin/sh -i'"#;
    if let Some(id) = container {
        if !asterion_protocol::is_lower_hex(id, 64) {
            return Err(io::Error::other("无效容器"));
        }
        command.arg(format!("exec /usr/bin/docker --host unix:///var/run/docker.sock exec -it --user 65534:65534 -- {id} {SHELL}"));
    } else {
        command.arg(format!(r#"test "$(id -u)" != 0 && exec {SHELL}"#));
    }
    command
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
            Ok(())
        });
    }
    let shell = command.spawn()?;
    drop(command);
    let initial = timeout(Duration::from_secs(6), ready(&master)).await??;
    let (tx, rx) = mpsc::channel(8);
    let output = Arc::new(Mutex::new(Output::default()));
    output
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(&initial);
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

async fn ready(master: &AsyncFd<File>) -> io::Result<Vec<u8>> {
    let marker = b"\x1eASTERION_READY\x1f";
    let mut received = Vec::new();
    let mut buffer = [0; 1024];
    while received.len() < MAX_READ {
        let count = master
            .async_io(Interest::READABLE, |mut f| f.read(&mut buffer))
            .await?;
        if count == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..count]);
        if let Some(at) = received
            .windows(marker.len())
            .position(|chunk| chunk == marker)
        {
            return Ok(received.split_off(at + marker.len()));
        }
    }
    Err(io::Error::other("SSH 终端未就绪"))
}

async fn drive(
    master: AsyncFd<File>,
    mut shell: Child,
    mut control: mpsc::Receiver<Control>,
    output: Arc<Mutex<Output>>,
) -> io::Result<()> {
    let mut exited = false;
    let mut absolute = Instant::now() + LIFETIME;
    let mut idle = Instant::now() + IDLE;
    let mut check_exit = interval(Duration::from_millis(100));
    check_exit.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut buffer = [0; MAX_READ];
    loop {
        tokio::select! {
            _ = sleep_until(absolute.min(idle)) => break,
            _ = check_exit.tick(), if !exited => {
                if shell.try_wait()?.is_some() {
                    exited = true;
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
            event = control.recv(), if !exited => match event {
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

fn success(value: Value) -> TaskReply {
    TaskReply::Success { value }
}
fn invalid() -> TaskReply {
    TaskReply::failure("invalid_request", "无效终端参数")
}
fn missing() -> TaskReply {
    TaskReply::failure("terminal_closed", "终端会话已结束")
}
