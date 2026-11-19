use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use asterion_protocol::managed::ManagedAction;
use serde_json::{Value, json};

use crate::{ExecError, gateway, run};

pub(crate) async fn query(
    sockets: &BTreeMap<String, PathBuf>,
    group: &str,
    key: Option<&str>,
) -> Result<Value, ExecError> {
    let socket = sockets.get(group).ok_or(ExecError::DENIED)?;
    if key.is_some_and(|key| !asterion_protocol::operation::valid_identifier(key, 64)) {
        return Err(ExecError::INVALID);
    }
    gateway::execute(
        socket,
        &json!({"query":true,"key":key}),
        Instant::now() + Duration::from_secs(10),
    )
    .await
}

pub(crate) async fn apply(
    socket: &Path,
    change: &ManagedAction,
    content: Option<&str>,
    expires_at: i64,
    deadline: Instant,
) -> Result<run::ResultData, ExecError> {
    gateway::execute(
        socket,
        &json!({"change":change,"content":content,"expires_at":expires_at}),
        deadline,
    )
    .await
}
