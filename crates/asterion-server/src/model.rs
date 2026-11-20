use asterion_protocol::NodeKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    Linux,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Docker,
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetManagement {
    Docker,
    Compose,
    Systemd,
    Process,
    Other,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub kind: HostKind,
    pub notes: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub host_id: Option<String>,
    pub name: String,
    pub kind: NodeKind,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub wsl_distro: Option<String>,
    pub notes: String,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Runtime {
    pub id: String,
    pub node_id: String,
    pub kind: RuntimeKind,
    pub identity: String,
    pub version: Option<String>,
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceAsset {
    pub id: String,
    pub node_id: String,
    pub name: String,
    pub management: AssetManagement,
    pub description: String,
    pub repo_url: Option<String>,
    pub deploy_path: Option<String>,
    pub config_path: Option<String>,
    pub tags: Vec<String>,
    pub notes: String,
    pub verified_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}
