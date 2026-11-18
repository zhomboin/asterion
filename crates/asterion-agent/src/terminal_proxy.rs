use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use asterion_protocol::{AgentReply, TaskRequest};
use serde_json::json;
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::terminal_wire;

pub struct TerminalProxy {
    path: Option<PathBuf>,
    connection: Option<UnixStream>,
}

impl TerminalProxy {
    pub fn new() -> Self {
        let path = std::env::var_os("ASTERION_TERMINAL_HELPER_SOCKET")
            .map(PathBuf::from)
            .filter(|path| {
                path.is_absolute()
                    && path.components().all(|part| {
                        !matches!(
                            part,
                            std::path::Component::ParentDir | std::path::Component::CurDir
                        )
                    })
            })
            .filter(|_| std::env::var("ASTERION_TERMINAL_ENABLED").as_deref() == Ok("1"));
        Self {
            path,
            connection: None,
        }
    }

    pub async fn handle(&mut self, request: TaskRequest) -> AgentReply {
        let retry_read = request.kind == "terminal_capability" && self.connection.is_some();
        if let Ok(Ok(reply)) = timeout(Duration::from_secs(2), self.exchange(&request)).await {
            return reply;
        }
        self.connection = None;
        // Only the capability query is safe to repeat after a previously open stream fails.
        if retry_read
            && let Ok(Ok(reply)) = timeout(Duration::from_secs(2), self.exchange(&request)).await
        {
            return reply;
        }
        self.connection = None;
        if request.kind == "terminal_capability" {
            AgentReply::Success {
                value: json!({"enabled":false}),
            }
        } else {
            AgentReply::failure("terminal_disabled", "独立终端服务当前不可用")
        }
    }

    async fn exchange(&mut self, request: &TaskRequest) -> io::Result<AgentReply> {
        if self.connection.is_none() {
            let path = self
                .path
                .as_ref()
                .ok_or_else(|| io::Error::other("terminal disabled"))?;
            let stream = UnixStream::connect(path).await?;
            let uid = stream.peer_cred()?.uid();
            if uid == 0 || uid == unsafe { libc::geteuid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "terminal helper must have a distinct unprivileged identity",
                ));
            }
            validate_endpoint(path, uid)?;
            self.connection = Some(stream);
        }
        let stream = self.connection.as_mut().expect("connection validated");
        terminal_wire::write(stream, request).await?;
        terminal_wire::read(stream).await
    }
}

fn validate_endpoint(path: &Path, uid: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent_stat = std::fs::symlink_metadata(parent)?;
    let endpoint = std::fs::symlink_metadata(path)?;
    if !parent_stat.is_dir()
        || parent_stat.uid() != uid
        || parent_stat.mode() & 0o022 != 0
        || !endpoint.file_type().is_socket()
        || endpoint.uid() != uid
    {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(())
}
