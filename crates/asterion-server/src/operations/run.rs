use std::sync::Arc;
use std::time::Duration;

use asterion_protocol::operation::OperationTask;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{SqliteConnection, SqlitePool};

use super::storage;
use crate::agent_bus::{AgentBus, BusError, PendingTask};

const DEADLINE: Duration = Duration::from_secs(75);
const OUTPUT_BYTES: usize = 32 * 1024;

pub(super) async fn execute(
    pool: SqlitePool,
    bus: Arc<AgentBus>,
    node: String,
    task: OperationTask,
) {
    let id = task.task_id.clone();
    let finished = match dispatch(&pool, &bus, &node, task).await {
        Ok(Some(Ok(pending))) => {
            let result = wait_with_progress(&pool, &id, pending).await;
            if result == Err(BusError::Timeout) {
                let _ = bus.cancel(&node, &id);
            }
            match result {
                Ok(output) => Outcome::from_output(output),
                Err(error) => Outcome::from_error(error),
            }
        }
        Ok(Some(Err(error))) => Outcome::from_error(error),
        Ok(None) => return,
        Err(_) => {
            let _ = bus.cancel(&node, &id);
            tracing::error!(task_id = %id, "运维任务状态写入失败");
            Outcome::with_error("interrupted", "operation_state_error")
        }
    };

    if finish(&pool, &id, finished).await.is_err() {
        tracing::error!(task_id = %id, "无法保存运维任务结果");
    }
}

async fn wait_with_progress(
    pool: &SqlitePool,
    id: &str,
    pending: PendingTask,
) -> Result<Value, BusError> {
    let mut updates = pending.progress();
    let result = pending.wait();
    tokio::pin!(result);
    let mut receiving_progress = true;

    loop {
        tokio::select! {
            reply = &mut result => {
                if reply.is_err() {
                    let last = updates.borrow_and_update().as_ref().map(|value| (value.output.clone(), value.truncated));
                    if let Some((output, truncated)) = last {
                        save_progress(pool, id, output, truncated).await;
                    }
                }
                return reply;
            }
            changed = updates.changed(), if receiving_progress => {
                receiving_progress = changed.is_ok();
                let snapshot = updates.borrow_and_update().as_ref().map(|value| (value.output.clone(), value.truncated));
                if let Some((output, truncated)) = snapshot {
                    save_progress(pool, id, output, truncated).await;
                }
            }
        }
    }
}

async fn save_progress(pool: &SqlitePool, id: &str, mut output: String, mut truncated: bool) {
    clamp_output(&mut output, &mut truncated);
    let saved = sqlx::query(
        "UPDATE operation_jobs SET output = ?, truncated = ? WHERE id = ? AND state = 'running'",
    )
    .bind(output)
    .bind(truncated)
    .bind(id)
    .execute(pool)
    .await;
    if saved.is_err() {
        tracing::warn!(task_id = %id, "无法保存运维任务部分输出");
    }
}

async fn dispatch(
    pool: &SqlitePool,
    bus: &AgentBus,
    node: &str,
    task: OperationTask,
) -> Result<Option<Result<PendingTask, BusError>>, sqlx::Error> {
    let id = task.task_id.clone();
    let payload = serde_json::to_value(task).map_err(sqlx::Error::decode)?;
    let mut tx = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE operation_jobs SET state = 'running', started_at = unixepoch()
         WHERE id = ? AND node_id = ? AND state = 'queued'",
    )
    .bind(&id)
    .bind(node)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        tx.commit().await?;
        return Ok(None);
    }
    // 取消与下发通过任务事务保持顺序。未进入 Agent 队列时不记录任务启动。
    let pending = match bus.enqueue(node, &id, "operation_execute", payload, DEADLINE) {
        Ok(pending) => pending,
        Err(error) => {
            tx.rollback().await?;
            return Ok(Some(Err(error)));
        }
    };
    storage::audit(&mut tx, &id, None, "dispatched").await?;
    tx.commit().await?;
    Ok(Some(Ok(pending)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    exit_code: Option<i32>,
    output: String,
    truncated: bool,
    duration_ms: u64,
}

struct Outcome {
    state: &'static str,
    error_code: Option<&'static str>,
    exit_code: Option<i32>,
    output: Option<String>,
    truncated: bool,
    duration_ms: Option<i64>,
}

impl Outcome {
    fn with_error(state: &'static str, error_code: &'static str) -> Self {
        Self {
            state,
            error_code: Some(error_code),
            exit_code: None,
            output: None,
            truncated: false,
            duration_ms: None,
        }
    }

    fn from_output(value: Value) -> Self {
        let Ok(mut output) = serde_json::from_value::<Output>(value) else {
            return Self::with_error("interrupted", "agent_invalid_result");
        };
        clamp_output(&mut output.output, &mut output.truncated);
        let succeeded = output.exit_code == Some(0);
        Self {
            state: if succeeded { "succeeded" } else { "failed" },
            error_code: (!succeeded).then_some("nonzero_exit"),
            exit_code: output.exit_code,
            output: Some(output.output),
            truncated: output.truncated,
            duration_ms: i64::try_from(output.duration_ms).ok(),
        }
    }

    fn from_error(error: BusError) -> Self {
        match error {
            BusError::Timeout => Self::with_error("timed_out", "agent_timeout"),
            BusError::Offline => Self::with_error("disconnected", "agent_offline"),
            BusError::Busy => Self::with_error("failed", "agent_busy"),
            BusError::Unsupported => Self::with_error("failed", "agent_upgrade_required"),
            BusError::Rejected { code, .. } => match code.as_str() {
                "cancelled" => Self::with_error("cancelled", "operator_cancelled"),
                "timeout" | "timed_out" => Self::with_error("timed_out", "agent_timeout"),
                "forbidden" | "permission_denied" | "operation_denied" => {
                    Self::with_error("failed", "agent_permission_denied")
                }
                "target_changed" | "target_mismatch" => {
                    Self::with_error("failed", "agent_target_changed")
                }
                _ => Self::with_error("failed", "agent_operation_rejected"),
            },
        }
    }
}

fn clamp_output(output: &mut String, truncated: &mut bool) {
    output.retain(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'));
    if output.len() > OUTPUT_BYTES {
        let mut boundary = OUTPUT_BYTES;
        while !output.is_char_boundary(boundary) {
            boundary -= 1;
        }
        output.truncate(boundary);
        *truncated = true;
    }
}

async fn finish(pool: &SqlitePool, id: &str, outcome: Outcome) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    finish_in_transaction(&mut tx, id, outcome).await?;
    tx.commit().await?;
    Ok(())
}

async fn finish_in_transaction(
    connection: &mut SqliteConnection,
    id: &str,
    outcome: Outcome,
) -> Result<(), sqlx::Error> {
    let truncated = outcome.output.as_ref().map(|_| outcome.truncated);
    let updated = sqlx::query(
        "UPDATE operation_jobs SET state = ?, error_code = ?, exit_code = ?,
         output = COALESCE(?, output), truncated = COALESCE(?, truncated), duration_ms = ?, finished_at = unixepoch()
         WHERE id = ? AND state IN ('queued', 'running')",
    )
    .bind(outcome.state)
    .bind(outcome.error_code)
    .bind(outcome.exit_code)
    .bind(outcome.output)
    .bind(truncated)
    .bind(outcome.duration_ms)
    .bind(id)
    .execute(&mut *connection)
    .await?;
    if updated.rows_affected() != 0 {
        storage::audit(connection, id, None, outcome.state).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use asterion_protocol::{AgentReply, TaskProgress, operation::OperationAction};

    use super::*;
    use crate::agent_bus::Control;

    #[tokio::test]
    async fn live_output_remains_available_if_the_agent_times_out() {
        let pool = crate::database::connect("sqlite::memory:").await.unwrap();
        let id = "1".repeat(32);
        let node = "2".repeat(32);
        let action = OperationAction::Command {
            name: "test-fixture".into(),
            parameters: Default::default(),
        };
        sqlx::query(
            "INSERT INTO operation_jobs (id, node_id, node_name, actor, session_hash,
             action, step_up_required, state, expires_at)
             VALUES (?, ?, '测试节点', 'test', zeroblob(32), ?, 0, 'queued', unixepoch() + 120)",
        )
        .bind(&id)
        .bind(&node)
        .bind(serde_json::to_string(&action).unwrap())
        .execute(&pool)
        .await
        .unwrap();
        let bus = Arc::new(AgentBus::new(15));
        let mut agent = bus.attach(&node, true);
        let simulate = async {
            let Control::Task(dispatch) = agent.next().await else {
                panic!("expected pending fixture");
            };
            dispatch
                .progress
                .send(Some(TaskProgress {
                    output: "部分输出\n".into(),
                    truncated: true,
                }))
                .unwrap();
            loop {
                let current: Option<String> = sqlx::query_scalar(
                    "SELECT output FROM operation_jobs WHERE id = ? AND state = 'running'",
                )
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
                if current.as_deref() == Some("部分输出\n") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            dispatch
                .response
                .send(AgentReply::failure("timeout", "fixture"))
                .unwrap();
        };

        let run = execute(
            pool.clone(),
            bus,
            node,
            OperationTask {
                task_id: id.clone(),
                action,
            },
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(simulate, run)
        })
        .await
        .unwrap();
        let result = sqlx::query_as::<_, (String, Option<String>, bool)>(
            "SELECT state, output, truncated FROM operation_jobs WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            result,
            ("timed_out".into(), Some("部分输出\n".into()), true)
        );
    }
}
