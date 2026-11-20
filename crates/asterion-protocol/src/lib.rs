pub mod managed;
pub mod operation;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_SAMPLE_INTERVAL_SECS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Linux,
    Wsl,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Wsl => "wsl",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRequest {
    pub request_id: String,
    pub kind: String,
    pub args: Value,
    pub expires_at: i64,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskReply {
    Success { value: Value },
    Failure { code: String, message: String },
}

impl TaskReply {
    pub fn failure(code: &str, message: &str) -> Self {
        Self::Failure {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

pub fn unix_time() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .try_into()
        .ok()
}

pub fn is_token(value: &str) -> bool {
    is_lower_hex(value, 64)
}

pub fn is_node_id(value: &str) -> bool {
    is_lower_hex(value, 32)
}

pub fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProgress {
    pub output: String,
    pub truncated: bool,
}
