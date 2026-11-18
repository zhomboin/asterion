use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CollectorError, MAX_QUERY_BYTES, bounded_result, parse_args};

mod engine;
mod transport;

const MAX_REQUEST_BYTES: usize = 1024;
const RESPONSE_OVERHEAD: usize = 128;
const MAX_LOG_LINES: u16 = 500;
const SOCKET_IDLE_TIMEOUT: Duration = Duration::from_millis(500);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "args",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Request {
    Docker,
    DockerStats(Container),
    DockerLogs(Logs),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Container {
    id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Logs {
    id: String,
    lines: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

impl Request {
    fn check(&self) -> Result<(), CollectorError> {
        match self {
            Self::Docker => return Ok(()),
            Self::DockerLogs(logs)
                if logs
                    .lines
                    .is_some_and(|lines| !(1..=MAX_LOG_LINES).contains(&lines)) =>
            {
                return Err(CollectorError::INVALID);
            }
            _ => {}
        }
        let id = match self {
            Self::Docker => unreachable!(),
            Self::DockerStats(container) => &container.id,
            Self::DockerLogs(logs) => &logs.id,
        };
        if id.len() == 64
            && id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(())
        } else {
            Err(CollectorError::INVALID)
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Response {
    Success { data: Value },
    Failure { code: String },
}

pub(crate) fn query(
    reader_path: Option<&Path>,
    kind: &str,
    args: Value,
) -> Result<Value, CollectorError> {
    let request = match kind {
        "docker" => {
            parse_args::<Empty>(args)?;
            Request::Docker
        }
        "docker_stats" => Request::DockerStats(parse_args(args)?),
        "docker_logs" => Request::DockerLogs(parse_args(args)?),
        _ => return Err(CollectorError::INVALID),
    };
    request.check()?;
    let inventory = matches!(request, Request::Docker);
    let Some(path) = reader_path else {
        return if inventory {
            Ok(engine::inventory_status("disabled"))
        } else {
            Err(CollectorError::new("disabled", "Docker 采集未启用"))
        };
    };

    match exchange(path, request) {
        Ok(data) => Ok(data),
        Err(CollectorError {
            code: "permission_denied",
            ..
        }) if inventory => Ok(engine::inventory_status("permission_denied")),
        Err(CollectorError {
            code: "unavailable" | "timeout",
            ..
        }) if inventory => Ok(engine::inventory_status("unreachable")),
        Err(error) => Err(error),
    }
}

fn exchange(path: &Path, request: Request) -> Result<Value, CollectorError> {
    let mut stream = transport::connect(path)?;
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let body = serde_json::to_vec(&request).map_err(|_| CollectorError::FAILED)?;
    send_line(&mut stream, &body, deadline)?;
    let line = read_line(stream, MAX_QUERY_BYTES + RESPONSE_OVERHEAD, deadline)?;
    let response: Response = serde_json::from_slice(&line).map_err(|_| CollectorError::FAILED)?;
    match response {
        Response::Success { data } => Ok(data),
        Response::Failure { code } => Err(match code.as_str() {
            "invalid_argument" => CollectorError::INVALID,
            "not_found" => CollectorError::NOT_FOUND,
            "permission_denied" => CollectorError::PERMISSION,
            "timeout" => CollectorError::TIMEOUT,
            "unavailable" => CollectorError::UNAVAILABLE,
            "output_too_large" => CollectorError::LARGE,
            "docker_error" => CollectorError::new("docker_error", "Docker 引擎未能处理读取请求"),
            _ => CollectorError::FAILED,
        }),
    }
}

pub fn serve(reader_path: &Path, engine_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if reader_path == engine_path || !reader_path.is_absolute() || !engine_path.is_absolute() {
        return Err("Docker 采集套接字必须为不同的本机绝对路径".into());
    }
    let parent = reader_path
        .parent()
        .ok_or("Docker 读取器套接字缺少父目录")?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir()
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err("Docker 读取器套接字父目录必须由服务账号持有并禁止其他账号写入".into());
    }
    let listener = UnixListener::bind(reader_path)?;
    std::fs::set_permissions(reader_path, std::fs::Permissions::from_mode(0o660))?;

    for socket in listener.incoming() {
        match socket {
            Ok(mut stream) => respond(&mut stream, engine_path),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn respond(stream: &mut UnixStream, engine_path: &Path) {
    let deadline = Instant::now() + REQUEST_TIMEOUT - SOCKET_IDLE_TIMEOUT;
    let result = (|| {
        let line = read_line(
            stream.try_clone().map_err(|_| CollectorError::FAILED)?,
            MAX_REQUEST_BYTES,
            Instant::now() + HANDSHAKE_TIMEOUT,
        )?;
        let request: Request =
            serde_json::from_slice(&line).map_err(|_| CollectorError::INVALID)?;
        request.check()?;
        let result = engine::query(engine_path, request, deadline)?;
        bounded_result(&result)?;
        Ok(result)
    })();
    let response = match result {
        Ok(data) => Response::Success { data },
        Err(CollectorError { code, .. }) => Response::Failure {
            code: code.to_owned(),
        },
    };
    if let Ok(body) = serde_json::to_vec(&response) {
        let _ = send_line(stream, &body, deadline + SOCKET_IDLE_TIMEOUT / 2);
    }
}

fn read_line(
    stream: UnixStream,
    maximum: usize,
    deadline: Instant,
) -> Result<Vec<u8>, CollectorError> {
    let stream = DeadlineReader { stream, deadline };
    let mut reader = BufReader::new(stream).take(maximum as u64 + 1);
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .map_err(transport::io_error)?;
    if line.len() > maximum {
        return Err(CollectorError::LARGE);
    }
    if line.last() != Some(&b'\n') {
        return Err(CollectorError::FAILED);
    }
    Ok(line)
}

struct DeadlineReader {
    stream: UnixStream,
    deadline: Instant,
}

impl Read for DeadlineReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(bytes)
    }
}

fn send_line(
    stream: &mut UnixStream,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), CollectorError> {
    for piece in [bytes, b"\n"] {
        let mut pending = piece;
        while !pending.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(CollectorError::TIMEOUT);
            }
            stream
                .set_write_timeout(Some(remaining))
                .map_err(transport::io_error)?;
            let count = stream.write(pending).map_err(transport::io_error)?;
            if count == 0 {
                return Err(CollectorError::UNAVAILABLE);
            }
            pending = &pending[count..];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_rejects_non_ids_and_large_log_tails() {
        let request = Request::DockerStats(Container {
            id: "../json".into(),
        });
        assert!(request.check().is_err());
        let request = Request::DockerLogs(Logs {
            id: "a".repeat(64),
            lines: Some(501),
        });
        assert!(request.check().is_err());
        assert!(
            serde_json::from_value::<Request>(
                serde_json::json!({"kind": "stop", "args":{"id":"a"}})
            )
            .is_err()
        );
    }

    #[test]
    fn unix_reader_only_returns_projected_data_from_a_fixture_engine() {
        use serde_json::json;
        use std::time::{SystemTime, UNIX_EPOCH};
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = repo
            .join(".local")
            .join(format!("dck-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let engine_path = dir.join("engine.sock");
        let reader_path = dir.join("reader.sock");
        let engine = UnixListener::bind(&engine_path).unwrap();
        let reader = UnixListener::bind(&reader_path).unwrap();
        let id = "b".repeat(64);
        let inspect_url = format!("/containers/{id}/json");
        let routes = [
            ("/version", json!({"Version":"fixture"})),
            (
                "/info",
                json!({"ID":"example-id","OperatingSystem":"Docker Desktop"}),
            ),
            (
                "/containers/json?all=1",
                json!([{"Id":id,"Names":["/demo"],"State":"running"}]),
            ),
            (
                inspect_url.as_str(),
                json!({"Config":{"Env":["TOKEN=sensitive"]},"RestartCount":4}),
            ),
        ];
        std::thread::scope(|scope| {
            let engine_job = scope.spawn(move || {
                for (path, payload) in routes {
                    let mut stream = accept_fixture(&engine);
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut inbound = BufReader::new(&stream);
                    let mut line = String::new();
                    inbound.read_line(&mut line).unwrap();
                    assert_eq!(line, format!("GET {path} HTTP/1.1\r\n"));
                    let body = serde_json::to_vec(&payload).unwrap();
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(headers.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            let reader_job = scope.spawn(move || {
                let mut stream = accept_fixture(&reader);
                respond(&mut stream, &engine_path);
            });
            let result = query(Some(&reader_path), "docker", Value::Null).unwrap();
            assert_eq!(result["engine_kind"], "desktop");
            assert_eq!(result["containers"][0]["restart_count"], 4);
            assert!(!result.to_string().contains("sensitive"));
            assert!(engine_job.join().is_ok() && reader_job.join().is_ok());
        });
    }

    fn accept_fixture(listener: &UnixListener) -> UnixStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        }
    }
}
