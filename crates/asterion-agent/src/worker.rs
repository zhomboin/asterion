use std::sync::{Arc, Mutex};

use asterion_collector::Collector;
use asterion_executor::Executor;
use asterion_protocol::operation::OperationTask;
use asterion_protocol::{AgentReply, TaskProgress, TaskRequest, unix_time};
use serde_json::Value;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{AbortHandle, JoinSet, spawn_blocking};

pub enum Completed {
    Metrics(Option<Value>),
    Task { id: String, reply: AgentReply },
}

pub struct Worker {
    collector: Arc<Mutex<Collector>>,
    permits: Arc<Semaphore>,
    executor: Arc<Executor>,
}

impl Worker {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            collector: Arc::new(Mutex::new(Collector::new())),
            permits: Arc::new(Semaphore::new(2)),
            executor: Arc::new(Executor::from_env()?),
        })
    }

    pub fn sample(&self, jobs: &mut JoinSet<Completed>) -> bool {
        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            return false;
        };
        let collector = Arc::clone(&self.collector);
        jobs.spawn(async move {
            let sample = spawn_blocking(move || {
                let _permit = permit;
                collector.lock().ok()?.metrics().ok()
            })
            .await
            .ok()
            .flatten();
            Completed::Metrics(sample)
        });
        true
    }

    pub fn task(
        &self,
        jobs: &mut JoinSet<Completed>,
        request: TaskRequest,
        progress: &mpsc::Sender<(String, TaskProgress)>,
    ) -> Option<AbortHandle> {
        if matches!(
            request.kind.as_str(),
            "operation_catalog" | "operation_execute" | "accounts" | "account_keys"
        ) {
            let executor = Arc::clone(&self.executor);
            let progress = progress.clone();
            return Some(jobs.spawn(async move {
                let id = request.request_id.clone();
                let reply = execute(executor, request, progress).await;
                Completed::Task { id, reply }
            }));
        }
        let permit = Arc::clone(&self.permits).try_acquire_owned().ok()?;
        let collector = Arc::clone(&self.collector);
        Some(jobs.spawn(async move {
            let TaskRequest {
                request_id,
                kind,
                args,
                expires_at,
            } = request;
            let reply = spawn_blocking(move || {
                let _permit = permit;
                let Ok(mut collector) = collector.lock() else {
                    return AgentReply::failure("collector_unavailable", "采集器未就绪");
                };
                if unix_time().is_none_or(|now| now >= expires_at) {
                    return AgentReply::failure("timeout", "查询已超时");
                }
                match collector.query(&kind, args) {
                    Ok(value) => AgentReply::Success { value },
                    Err(err) => AgentReply::failure(err.code, err.message),
                }
            })
            .await
            .unwrap_or_else(|_| AgentReply::failure("collector_unavailable", "采集器异常"));
            Completed::Task {
                id: request_id,
                reply,
            }
        }))
    }
}

async fn execute(
    executor: Arc<Executor>,
    request: TaskRequest,
    progress: mpsc::Sender<(String, TaskProgress)>,
) -> AgentReply {
    if request.kind == "accounts" {
        return match spawn_blocking(move || executor.accounts()).await {
            Ok(result) => reply(result),
            Err(_) => AgentReply::failure("unavailable", "读取节点账号失败"),
        };
    }
    if request.kind == "account_keys" {
        let Some(username) = request.args.get("username").and_then(Value::as_str) else {
            return AgentReply::failure("invalid_input", "用户名格式错误");
        };
        return reply(executor.account_keys(username).await);
    }
    if request.kind == "operation_catalog" {
        return AgentReply::Success {
            value: executor.catalog(),
        };
    }
    let Ok(task) = serde_json::from_value::<OperationTask>(request.args) else {
        return AgentReply::failure("invalid_input", "运维动作格式错误");
    };
    if task.task_id != request.request_id {
        return AgentReply::failure("invalid_input", "运维任务标识不一致");
    }
    let id = request.request_id;
    reply(
        executor
            .execute_with_progress(task, request.expires_at, |update| {
                let _ = progress.try_send((
                    id.clone(),
                    TaskProgress {
                        output: update.output,
                        truncated: update.truncated,
                    },
                ));
            })
            .await,
    )
}

fn reply(result: Result<Value, asterion_executor::ExecError>) -> AgentReply {
    match result {
        Ok(value) => AgentReply::Success { value },
        Err(err) => AgentReply::failure(err.code, err.message),
    }
}
