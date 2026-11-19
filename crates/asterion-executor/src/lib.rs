use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use asterion_protocol::operation::{
    AccountChange, OperationAction, OperationTask, ProcessSignal, ServiceVerb,
};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

mod accounts;
mod child_group;
mod docker;
mod gateway;
mod managed;
mod policy;
mod run;
mod signal;

pub use accounts::operator::serve as serve_account_operator;
pub use child_group::{confine_process_group, leader_exited};
pub use docker::serve as serve_docker_operator;
pub use run::RunProgress;

const MAX_DEADLINE_SECONDS: u64 = 180;
const MAX_RECENT_TASKS: usize = 1024;

pub struct Executor {
    policy: policy::Policy,
    running: Semaphore,
    recent: Mutex<BTreeMap<String, Instant>>,
}

#[derive(Debug, Copy, Clone)]
pub struct ExecError {
    pub code: &'static str,
    pub message: &'static str,
}

impl ExecError {
    pub(crate) const INVALID: Self = Self::new("invalid_argument", "运维参数无效");
    pub(crate) const DENIED: Self = Self::new("operation_denied", "节点本地策略未授权此操作");
    pub(crate) const DISABLED: Self = Self::new("disabled", "节点未启用受控操作");
    pub(crate) const TIMEOUT: Self = Self::new("timeout", "运维操作已超时");
    pub(crate) const POLICY: Self = Self::new("invalid_policy", "节点本地运维策略文件无效");
    pub(crate) const FAILED: Self = Self::new("execution_failed", "节点未能完成运维操作");
    pub(crate) const REPLAY: Self = Self::new("task_replayed", "任务编号已处理");
    pub(crate) const BUSY: Self = Self::new("agent_busy", "节点已有运维任务正在执行");
    pub(crate) const STALE_TARGET: Self = Self::new("target_changed", "目标进程已退出或身份已改变");

    pub(crate) const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::PermissionDenied => Self::DENIED,
            std::io::ErrorKind::TimedOut => Self::TIMEOUT,
            _ => Self::FAILED,
        }
    }
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ExecError {}

impl Executor {
    pub fn from_env() -> Result<Self, ExecError> {
        let policy = std::env::var_os("ASTERION_EXECUTOR_CONFIG_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .map(|path| policy::Policy::from_file(&path))
            .transpose()?
            .unwrap_or_default();
        Ok(Self::new(policy))
    }

    fn new(policy: policy::Policy) -> Self {
        Self {
            policy,
            running: Semaphore::new(1),
            recent: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn catalog(&self) -> Value {
        self.policy.catalog()
    }

    pub async fn managed_query(&self, group: &str, key: Option<&str>) -> Result<Value, ExecError> {
        managed::query(&self.policy.managed, group, key).await
    }

    pub fn accounts(&self) -> Result<Value, ExecError> {
        accounts::inventory()
    }

    pub async fn account_keys(&self, username: &str) -> Result<Value, ExecError> {
        if !asterion_protocol::operation::valid_username(username) {
            return Err(ExecError::INVALID);
        }
        if let Some(policy) = &self.policy.accounts
            && policy.usernames.iter().any(|name| name == username)
        {
            return accounts::operator::remote_keys(&policy.operator_socket, username).await;
        }
        accounts::key_inventory(username)
    }

    pub async fn execute(&self, task: OperationTask, expires_at: i64) -> Result<Value, ExecError> {
        self.execute_inner(task, expires_at, None).await
    }

    pub async fn execute_with_progress(
        &self,
        task: OperationTask,
        expires_at: i64,
        progress: impl Fn(RunProgress) + Send + Sync,
    ) -> Result<Value, ExecError> {
        self.execute_inner(task, expires_at, Some(&progress)).await
    }

    async fn execute_inner(
        &self,
        task: OperationTask,
        expires_at: i64,
        progress: Option<run::ProgressCallback<'_>>,
    ) -> Result<Value, ExecError> {
        if !self.policy.enabled() {
            return Err(ExecError::DISABLED);
        }
        if !asterion_protocol::is_lower_hex(&task.task_id, 32) || !task.action.valid() {
            return Err(ExecError::INVALID);
        }
        let remaining = if matches!(task.action, OperationAction::Managed { .. }) {
            bounded_budget(expires_at, 930)?
        } else {
            time_budget(expires_at)?
        };
        let deadline = Instant::now() + remaining;
        let operation = self.authorize(&task.action)?;
        let _slot = self.running.try_acquire().map_err(|_| ExecError::BUSY)?;
        self.claim(task.task_id, remaining).await?;
        let result = match operation {
            Authorized::Managed(socket, change) => {
                managed::apply(
                    socket,
                    change,
                    task.content.as_deref(),
                    expires_at,
                    deadline,
                )
                .await?
            }
            Authorized::Command(program, args, timeout) => {
                let mut outcome = run::command_with_progress(
                    program,
                    &args,
                    deadline.min(Instant::now() + timeout),
                    progress,
                )
                .await?;
                if let OperationAction::Systemd { unit, verb } = &task.action
                    && outcome.exit_code == Some(0)
                {
                    let state = run::command(
                        program,
                        &[
                            "show".into(),
                            "--property=ActiveState".into(),
                            "--value".into(),
                            "--".into(),
                            unit.clone(),
                        ],
                        deadline,
                    )
                    .await?;
                    let active = state.output.trim();
                    let expected = match verb {
                        ServiceVerb::Stop => active == "inactive",
                        _ => matches!(active, "active" | "reloading"),
                    };
                    if state.exit_code != Some(0) || !expected {
                        outcome.exit_code = Some(1);
                        outcome
                            .output
                            .push_str("服务未达到预期状态，请核对服务详情");
                    } else {
                        outcome.output.push_str(&format!("服务状态：{active}"));
                    }
                }
                outcome
            }
            Authorized::Docker {
                socket,
                engine_id,
                container_id,
                verb,
            } => {
                docker::execute(socket, engine_id, container_id, verb, expires_at, deadline).await?
            }
            Authorized::Account { socket, change } => {
                accounts::operator::execute(socket, change, expires_at, deadline).await?
            }
            Authorized::Process {
                pid,
                start_ticks,
                signal,
            } => signal::signal_process(pid, start_ticks, signal)?,
        };
        serde_json::to_value(result).map_err(|_| ExecError::FAILED)
    }

    async fn claim(&self, task_id: String, remaining: Duration) -> Result<(), ExecError> {
        let mut recent = self.recent.lock().await;
        let now = Instant::now();
        recent.retain(|_, expiry| *expiry > now);
        if recent.contains_key(&task_id) {
            return Err(ExecError::REPLAY);
        }
        if recent.len() >= MAX_RECENT_TASKS {
            return Err(ExecError::BUSY);
        }
        // Keep completed IDs through their maximum redelivery window.
        recent.insert(
            task_id,
            now + remaining + Duration::from_secs(MAX_DEADLINE_SECONDS),
        );
        Ok(())
    }

    fn authorize<'a>(&'a self, action: &'a OperationAction) -> Result<Authorized<'a>, ExecError> {
        match action {
            OperationAction::Managed { change } => {
                let socket = self
                    .policy
                    .managed
                    .get(change.verb.kind())
                    .ok_or(ExecError::DENIED)?;
                Ok(Authorized::Managed(socket, change))
            }
            OperationAction::Systemd { unit, verb } if self.policy.systemd_units.contains(unit) => {
                Ok(Authorized::Command(
                    Path::new("/usr/bin/systemctl"),
                    vec![
                        "--system".into(),
                        "--no-ask-password".into(),
                        "--no-pager".into(),
                        verb_name(*verb).into(),
                        "--".into(),
                        unit.clone(),
                    ],
                    Duration::from_secs(30),
                ))
            }
            OperationAction::Docker {
                engine_id,
                container_id,
                verb,
            } => {
                let docker = self.policy.docker.as_ref().ok_or(ExecError::DENIED)?;
                if !policy::allowed_container(&docker.targets, engine_id, container_id) {
                    return Err(ExecError::DENIED);
                }
                Ok(Authorized::Docker {
                    socket: &docker.operator_socket,
                    engine_id,
                    container_id,
                    verb: *verb,
                })
            }
            OperationAction::Process {
                pid,
                start_ticks,
                signal,
            } if match signal {
                ProcessSignal::Term => self.policy.process.term,
                ProcessSignal::Kill => self.policy.process.kill,
            } =>
            {
                Ok(Authorized::Process {
                    pid: *pid,
                    start_ticks: *start_ticks,
                    signal: *signal,
                })
            }
            OperationAction::Account { change } => {
                let policy = self.policy.accounts.as_ref().ok_or(ExecError::DENIED)?;
                if !policy.allows(change) {
                    return Err(ExecError::DENIED);
                }
                Ok(Authorized::Account {
                    socket: &policy.operator_socket,
                    change,
                })
            }
            OperationAction::Command { name, parameters } => {
                let command = self
                    .policy
                    .commands
                    .iter()
                    .find(|entry| entry.name == *name)
                    .ok_or(ExecError::DENIED)?;
                Ok(Authorized::Command(
                    &command.program,
                    command.arguments(parameters)?,
                    Duration::from_secs(command.timeout_seconds),
                ))
            }
            _ => Err(ExecError::DENIED),
        }
    }
}

enum Authorized<'a> {
    Managed(&'a Path, &'a asterion_protocol::managed::ManagedAction),
    Command(&'a Path, Vec<String>, Duration),
    Docker {
        socket: &'a Path,
        engine_id: &'a str,
        container_id: &'a str,
        verb: ServiceVerb,
    },
    Process {
        pid: u32,
        start_ticks: u64,
        signal: ProcessSignal,
    },
    Account {
        socket: &'a Path,
        change: &'a AccountChange,
    },
}

pub(crate) fn time_budget(expires_at: i64) -> Result<Duration, ExecError> {
    bounded_budget(expires_at, MAX_DEADLINE_SECONDS)
}

fn bounded_budget(expires_at: i64, limit: u64) -> Result<Duration, ExecError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ExecError::FAILED)?
        .as_secs();
    let expires_at = u64::try_from(expires_at).map_err(|_| ExecError::INVALID)?;
    let seconds = expires_at
        .checked_sub(now)
        .filter(|seconds| *seconds > 0)
        .ok_or(ExecError::TIMEOUT)?;
    if seconds > limit {
        return Err(ExecError::INVALID);
    }
    Ok(Duration::from_secs(seconds))
}

pub(crate) fn verb_name(verb: ServiceVerb) -> &'static str {
    match verb {
        ServiceVerb::Start => "start",
        ServiceVerb::Stop => "stop",
        ServiceVerb::Restart => "restart",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_recent_task_replay() {
        let executor = Executor::new(policy::Policy::default());
        let lifetime = Duration::from_secs(1);
        executor.claim("a".repeat(32), lifetime).await.unwrap();
        let error = executor.claim("a".repeat(32), lifetime).await.unwrap_err();
        assert_eq!(error.code, "task_replayed");
        assert_eq!(executor.catalog()["enabled"], false);
    }
}
