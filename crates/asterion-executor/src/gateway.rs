use std::future::Future;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::{ExecError, policy};

const MAX_REQ_BYTES: usize = 52 * 1024;
const MAX_RESPONSE_BYTES: usize = 96 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum WireResponse<R> {
    Success { result: R },
    Failure { error: String },
}

pub(crate) async fn execute<R: DeserializeOwned>(
    address: &Path,
    request: &impl Serialize,
    deadline: Instant,
) -> Result<R, ExecError> {
    let command = async {
        let stream = UnixStream::connect(address).await.map_err(ExecError::io)?;
        let mut stream = BufReader::new(stream);
        write_message(stream.get_mut(), request, MAX_REQ_BYTES).await?;
        let response: WireResponse<R> = read_message(&mut stream, MAX_RESPONSE_BYTES).await?;
        match response {
            WireResponse::Success { result } => Ok(result),
            WireResponse::Failure { error } => Err(match error.as_str() {
                "invalid_argument" => ExecError::INVALID,
                "operation_denied" => ExecError::DENIED,
                "permission_denied" => {
                    ExecError::new("permission_denied", "无法读取或维护此公钥文件")
                }
                "target_changed" => ExecError::STALE_TARGET,
                "timeout" => ExecError::TIMEOUT,
                "task_replayed" => ExecError::REPLAY,
                "agent_busy" => ExecError::BUSY,
                "not_found" => ExecError::new("not_found", "目标不存在"),
                "conflict" => ExecError::new("conflict", "目标已存在或已发生变化"),
                _ => ExecError::FAILED,
            }),
        }
    };
    tokio::time::timeout_at(deadline.into(), command)
        .await
        .map_err(|_| ExecError::TIMEOUT)?
}

pub(crate) async fn serve<T, R, F, Fut>(address: &Path, handler: F) -> Result<(), ExecError>
where
    T: DeserializeOwned,
    R: Serialize,
    F: Fn(T) -> Fut,
    Fut: Future<Output = Result<R, ExecError>>,
{
    policy::validate_socket_path(address)?;
    let parent = address.parent().ok_or(ExecError::POLICY)?;
    let metadata = std::fs::symlink_metadata(parent).map_err(ExecError::io)?;
    if !metadata.is_dir()
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(ExecError::POLICY);
    }
    let listener = UnixListener::bind(address).map_err(ExecError::io)?;
    let file_mode = std::fs::Permissions::from_mode(0o660);
    std::fs::set_permissions(address, file_mode).map_err(ExecError::io)?;

    loop {
        let (stream, _) = listener.accept().await.map_err(ExecError::io)?;
        let mut stream = BufReader::new(stream);
        let request =
            tokio::time::timeout(HEADER_TIMEOUT, read_message(&mut stream, MAX_REQ_BYTES)).await;
        let result = match request {
            Ok(Ok(request)) => tokio::select! {
                biased;
                _ = stream.read_u8() => continue,
                outcome = handler(request) => outcome,
            },
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ExecError::TIMEOUT),
        };

        let response = match result {
            Ok(result) => WireResponse::Success { result },
            Err(error) => WireResponse::Failure {
                error: error.code.to_owned(),
            },
        };
        let _ = tokio::time::timeout(
            HEADER_TIMEOUT,
            write_message(stream.get_mut(), &response, MAX_RESPONSE_BYTES),
        )
        .await;
    }
}

async fn read_message<T: DeserializeOwned>(
    stream: &mut BufReader<UnixStream>,
    limit: usize,
) -> Result<T, ExecError> {
    let mut line = Vec::new();
    stream
        .take(limit as u64 + 1)
        .read_until(b'\n', &mut line)
        .await
        .map_err(ExecError::io)?;
    if line.len() > limit || line.last() != Some(&b'\n') {
        return Err(ExecError::INVALID);
    }
    serde_json::from_slice(&line).map_err(|_| ExecError::INVALID)
}

async fn write_message(
    stream: &mut UnixStream,
    value: &impl Serialize,
    limit: usize,
) -> Result<(), ExecError> {
    let data = serde_json::to_vec(value).map_err(|_| ExecError::FAILED)?;
    if data.len() >= limit {
        return Err(ExecError::INVALID);
    }
    stream.write_all(&data).await.map_err(ExecError::io)?;
    stream.write_all(b"\n").await.map_err(ExecError::io)
}
