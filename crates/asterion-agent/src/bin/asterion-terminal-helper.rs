use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use asterion_protocol::{AgentReply, TaskRequest, unix_time};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::timeout;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
// The shell's input-idle limit is two minutes and active clients poll regularly.
const REQUEST_IDLE_TIMEOUT: Duration = Duration::from_secs(150);

#[path = "../terminal.rs"]
mod terminal;
#[path = "../terminal_wire.rs"]
mod terminal_wire;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    ensure!(uid != 0, "终端服务必须使用独立的非 root 账号");
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } < 0 {
        return Err(std::io::Error::last_os_error()).context("无法保护终端服务进程");
    }
    let path = std::env::var_os("ASTERION_TERMINAL_HELPER_SOCKET")
        .map(PathBuf::from)
        .context("请配置 ASTERION_TERMINAL_HELPER_SOCKET")?;
    let peer_uid = std::env::var("ASTERION_TERMINAL_AGENT_UID")
        .ok()
        .map(|value| value.parse::<u32>())
        .transpose()
        .context("ASTERION_TERMINAL_AGENT_UID 无效")?;
    ensure!(
        peer_uid.is_none_or(|peer| peer != uid && peer != 0),
        "Agent 与终端服务账号必须独立"
    );
    prepare(&path, uid, peer_uid.is_some()).await?;
    let listener = UnixListener::bind(&path)?;
    let metadata = std::fs::symlink_metadata(&path)?;
    let _socket = SocketPath {
        path: path.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let mode = if peer_uid.is_some() { 0o666 } else { 0o660 };
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = terminate.recv() => Ok(()),
        result = tokio::signal::ctrl_c() => result.map_err(Into::into),
        result = serve(listener, uid, peer_uid) => result,
    }
}

async fn serve(listener: UnixListener, uid: u32, peer_uid: Option<u32>) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let Ok(credential) = stream.peer_cred() else {
            continue;
        };
        if credential.uid() == uid
            || credential.uid() == 0
            || peer_uid.is_some_and(|peer| peer != credential.uid())
        {
            continue;
        }
        let _ = session(stream).await;
    }
}

async fn session(mut stream: UnixStream) -> std::io::Result<()> {
    let mut terminal = terminal::Terminals::new();
    let mut idle_limit = HANDSHAKE_TIMEOUT;
    loop {
        let request: TaskRequest = timeout(idle_limit, terminal_wire::read(&mut stream))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        let reply = if unix_time()
            .is_some_and(|now| (1..=120).contains(&request.expires_at.saturating_sub(now)))
        {
            terminal.handle(request)
        } else {
            AgentReply::failure("timeout", "终端请求已过期")
        };
        timeout(HANDSHAKE_TIMEOUT, terminal_wire::write(&mut stream, &reply))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        idle_limit = REQUEST_IDLE_TIMEOUT;
    }
}

async fn prepare(path: &Path, uid: u32, peer_restricted: bool) -> Result<()> {
    ensure!(path.is_absolute(), "终端 Socket 必须为绝对路径");
    let directory = path.parent().context("终端 Socket 缺少父目录")?;
    let metadata = std::fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "终端 Socket 父目录须归本服务所有并禁止其他用户写入"
    );
    ensure!(
        peer_restricted || metadata.mode() & 0o007 == 0,
        "请将终端 Socket 父目录限制为服务账号与授权组"
    );
    let existing = match std::fs::symlink_metadata(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !existing.file_type().is_socket() || existing.uid() != uid {
        bail!("终端 Socket 路径已被其他文件占用");
    }
    match timeout(HANDSHAKE_TIMEOUT, UnixStream::connect(path)).await {
        Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
        Ok(Err(error)) => return Err(error.into()),
        _ => bail!("已有终端服务占用该 Socket"),
    }
    let stale = SocketPath {
        path: path.to_owned(),
        device: existing.dev(),
        inode: existing.ino(),
    };
    if stale.still_owned() {
        std::fs::remove_file(path)?;
    }
    // The guard only removes the examined inode, even when another service already bound the path.
    Ok(())
}

struct SocketPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl SocketPath {
    fn still_owned(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        })
    }
}

impl Drop for SocketPath {
    fn drop(&mut self) {
        if self.still_owned() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn incomplete_handshake_cannot_hold_the_helper() {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_u8(0).await.unwrap();
        let outcome = session(server).await.unwrap_err();
        assert_eq!(outcome.kind(), io::ErrorKind::TimedOut);
    }
}
