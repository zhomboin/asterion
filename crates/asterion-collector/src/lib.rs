use serde::de::DeserializeOwned;
use serde_json::{Value, json};

mod docker;
mod mounts;
mod processes;
mod redact;
mod system;
mod systemd;

pub use docker::serve as serve_docker_reader;
pub use processes::process_start_ticks;
pub use redact::line as redact_output_line;

const MAX_QUERY_BYTES: usize = 192 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct CollectorError {
    pub code: &'static str,
    pub message: &'static str,
}

impl CollectorError {
    pub(crate) const INVALID: Self = Self::new("invalid_argument", "采集参数无效");
    pub(crate) const FAILED: Self = Self::new("collection_failed", "本地采集失败");
    pub(crate) const UNAVAILABLE: Self = Self::new("unavailable", "采集功能不可用");
    pub(crate) const PERMISSION: Self = Self::new("permission_denied", "采集权限不足");
    pub(crate) const NOT_FOUND: Self = Self::new("not_found", "采集目标不存在");
    pub(crate) const TIMEOUT: Self = Self::new("timeout", "本地采集超时");
    pub(crate) const LARGE: Self = Self::new("output_too_large", "采集结果超出上限");

    pub(crate) const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for CollectorError {}

pub struct Collector {
    system: system::Sampler,
    processes: processes::Sampler,
    docker_reader: Option<std::path::PathBuf>,
}

impl Collector {
    pub fn new() -> Self {
        Self {
            system: system::Sampler::default(),
            processes: processes::Sampler::default(),
            docker_reader: std::env::var_os("ASTERION_DOCKER_READER_SOCKET")
                .filter(|value| !value.is_empty())
                .map(Into::into),
        }
    }

    pub fn metrics(&mut self) -> Result<Value, CollectorError> {
        serde_json::to_value(self.system.sample()?).map_err(|_| CollectorError::FAILED)
    }

    pub fn query(&mut self, kind: &str, args: Value) -> Result<Value, CollectorError> {
        let result = match kind {
            "processes" => self.processes.list(parse_args(args)?)?,
            "process" => self.processes.detail(parse_args(args)?)?,
            "systemd" => systemd::list(parse_args(args)?)?,
            "systemd_service" => systemd::detail(parse_args(args)?)?,
            "docker" | "docker_stats" | "docker_logs" => {
                docker::query(self.docker_reader.as_deref(), kind, args)?
            }
            _ => return Err(CollectorError::INVALID),
        };
        bounded_result(&result)?;
        Ok(result)
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, CollectorError> {
    serde_json::from_value(if args.is_null() { json!({}) } else { args })
        .map_err(|_| CollectorError::INVALID)
}

pub(crate) fn bounded_result(result: &Value) -> Result<(), CollectorError> {
    if serde_json::to_vec(result)
        .map_err(|_| CollectorError::FAILED)?
        .len()
        > MAX_QUERY_BYTES
    {
        return Err(CollectorError::LARGE);
    }
    Ok(())
}

pub(crate) fn observed_at() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

pub(crate) fn read_file(path: impl AsRef<std::path::Path>, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
