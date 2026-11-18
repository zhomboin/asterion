pub mod operation;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u16 = 2;
pub const MIN_PROTOCOL_VERSION: u16 = 1;
pub const AGENT_CONNECT_PATH: &str = "/api/v1/agent/connect";
pub const ENROLLMENT_HEADER: &str = "x-asterion-enrollment";
pub const HEARTBEAT_INTERVAL_SECS: u64 = 5;
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 20;
pub const MAX_MESSAGE_SIZE: usize = 256 * 1024;
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
pub struct Envelope<T> {
    pub protocol_version: u16,
    pub node_id: Option<String>,
    pub message_id: u64,
    pub timestamp: i64,
    #[serde(flatten)]
    pub message: T,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentMessage {
    Hello(Hello),
    Heartbeat {},
    Metrics {
        sample: Value,
    },
    TaskReply {
        request_id: String,
        reply: AgentReply,
    },
    TaskProgress {
        request_id: String,
        progress: TaskProgress,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ServerMessage {
    Welcome {},
    HeartbeatAck {},
    Configure { sample_interval_secs: u64 },
    Task(TaskRequest),
    CancelTask { request_id: String },
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
pub enum AgentReply {
    Success { value: Value },
    Failure { code: String, message: String },
}

impl AgentReply {
    pub fn failure(code: &str, message: &str) -> Self {
        Self::Failure {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub agent_version: String,
    pub os: String,
    pub arch: String,
    pub wsl_distro: Option<String>,
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
