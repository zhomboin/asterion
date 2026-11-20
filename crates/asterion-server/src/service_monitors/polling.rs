use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::storage::{self, Observation};
use crate::ssh_bus::{BusError, SshBus};

const MAX_PARALLEL_NODES: i64 = 8;
const CHECKS_PER_NODE: i64 = 8;
const SOFT_BATCH_BUDGET: Duration = Duration::from_secs(5);

pub async fn poll_due(pool: &SqlitePool, bus: &Arc<SshBus>) -> anyhow::Result<()> {
    let mut by_node = BTreeMap::<_, Vec<_>>::new();
    for monitor in storage::due(pool, MAX_PARALLEL_NODES, CHECKS_PER_NODE).await? {
        by_node
            .entry(monitor.node_id.clone())
            .or_default()
            .push(monitor);
    }
    let mut jobs = JoinSet::new();
    for (node, monitors) in by_node {
        let pool = pool.clone();
        let bus = Arc::clone(bus);
        jobs.spawn(async move {
            // SSH 的采集器每次只执行一个查询，避免轮询独占同节点的工作槽。
            let deadline = Instant::now() + SOFT_BATCH_BUDGET;
            for monitor in monitors {
                let outcome = bus
                    .query(&node, "systemd_service", json!({ "unit": monitor.unit }))
                    .await;
                let unavailable = matches!(&outcome, Err(BusError::Busy | BusError::Timeout));
                let observation = interpret(&monitor.unit, outcome);
                storage::record(&pool, &monitor, &observation).await?;
                if unavailable || Instant::now() >= deadline {
                    break;
                }
            }
            Ok::<(), sqlx::Error>(())
        });
    }
    let mut failure = None;
    while let Some(result) = jobs.join_next().await {
        let result = result
            .map_err(anyhow::Error::from)
            .and_then(|result| result.map_err(Into::into));
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

pub(super) fn interpret(unit: &str, outcome: Result<Value, BusError>) -> Observation {
    let value = match outcome {
        Ok(value) => value,
        Err(error) => return Observation::unknown(failure_reason(error)),
    };
    if value.get("name").and_then(Value::as_str) != Some(unit)
        || value.get("source").and_then(Value::as_str) != Some("systemd")
        || value.get("verified").and_then(Value::as_bool) != Some(true)
    {
        return Observation::unknown("invalid_response");
    }
    let (Some(active), Some(sub)) = (field(&value, "active"), field(&value, "sub")) else {
        return Observation::unknown("invalid_response");
    };
    let (status, reason) = match active {
        "active" | "reloading" | "refreshing" => ("healthy", None),
        "failed" | "inactive" => ("down", None),
        _ => ("unknown", Some("transition")),
    };
    Observation {
        status,
        active: Some(active.to_owned()),
        sub: Some(sub.to_owned()),
        reason,
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value.get(name).and_then(Value::as_str).filter(|field| {
        !field.is_empty()
            && field.len() <= 32
            && field
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn failure_reason(error: BusError) -> &'static str {
    match error {
        BusError::Offline => "offline",
        BusError::Busy => "busy",
        BusError::Timeout => "timeout",
        BusError::Unsupported => "unsupported",
        BusError::Rejected { code, .. } => match code.as_str() {
            "not_found" => "not_found",
            "permission_denied" => "permission_denied",
            "unavailable" | "unreachable" | "not_installed" | "disabled" => "unavailable",
            _ => "query_failed",
        },
    }
}
