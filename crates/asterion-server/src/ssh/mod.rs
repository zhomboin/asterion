mod config;
mod direct;
mod pty;
mod transport;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use asterion_protocol::{TaskReply, unix_time};
use axum::{Json, http::StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Semaphore, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{MissedTickBehavior, interval};

use crate::auth::AdminSession;
use crate::ssh_bus::{Control, Dispatch, SshBus};
use config::Target;
pub(crate) use config::valid_profile;

pub(crate) fn valid_connection(value: &str, local: bool) -> bool {
    valid_profile(value) || local && direct::Direct::parse(value).is_some()
}

pub(crate) async fn profiles(
    _: AdminSession,
    local: Option<axum::Extension<crate::local::LocalPanel>>,
) -> Result<Json<Vec<String>>, (StatusCode, Json<Value>)> {
    config::load(local.is_some())
        .await
        .map(|p| Json(p.into_keys().filter(|name| valid_profile(name)).collect()))
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"message":"中心 SSH 配置不可读或格式无效"})),
            )
        })
}

struct Worker {
    target: Target,
    task: JoinHandle<()>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(pool: SqlitePool, bus: Arc<SshBus>, mut shutdown: watch::Receiver<bool>) {
    let mut workers: HashMap<String, Worker> = HashMap::new();
    let slots = Arc::new(Semaphore::new(32));
    let mut tick = interval(Duration::from_secs(3));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {}
        }
        let mut profiles = config::load(true).await.unwrap_or_default();
        let Ok(nodes) = sqlx::query_as::<_, (String, String)>(
            "SELECT id, ssh_profile FROM nodes WHERE ssh_profile IS NOT NULL",
        )
        .fetch_all(&pool)
        .await
        else {
            continue;
        };
        for (_, name) in &nodes {
            if let Some(direct) = direct::Direct::parse(name) {
                profiles
                    .entry(name.clone())
                    .or_insert_with(|| Target::Direct(direct, Value::Null));
            }
        }
        workers.retain(|id, worker| {
            !worker.task.is_finished()
                && nodes.iter().any(|(node, profile)| {
                    node == id && profiles.get(profile) == Some(&worker.target)
                })
        });
        for (id, name) in nodes {
            if workers.contains_key(&id) {
                continue;
            }
            let Some(target) = profiles.get(&name) else {
                let _ = sqlx::query("UPDATE nodes SET ssh_error='中心找不到此 SSH 配置', ssh_online_until=MIN(COALESCE(ssh_online_until,unixepoch()),unixepoch()) WHERE id=? AND ssh_error IS NOT '中心找不到此 SSH 配置'")
                    .bind(&id).execute(&pool).await;
                continue;
            };
            let task = tokio::spawn(site(
                pool.clone(),
                bus.clone(),
                slots.clone(),
                id.clone(),
                target.clone(),
            ));
            workers.insert(
                id,
                Worker {
                    target: target.clone(),
                    task,
                },
            );
        }
    }
}

async fn site(
    pool: SqlitePool,
    bus: Arc<SshBus>,
    slots: Arc<Semaphore>,
    id: String,
    target: Target,
) {
    let mut subscription = bus.attach(&id, true);
    let mut jobs = JoinSet::new();
    let mut cancellations = HashMap::new();
    let terminals = Arc::new(Mutex::new(pty::Terminals::new()));
    let mut tick = interval(Duration::from_secs(bus.sample_interval_secs()));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut sampling = false;
    loop {
        tokio::select! {
            _ = tick.tick(), if !sampling => {
                sampling = true;
                let slots = slots.clone();
                let pool = pool.clone();
                let id = id.clone();
                let target = target.clone();
                let period = bus.sample_interval_secs();
                jobs.spawn(async move {
                    if let Ok(_permit) = slots.acquire().await {
                        sample(&pool, &id, &target, period).await;
                    }
                    None
                });
            }
            Some(Ok(done)) = jobs.join_next(), if !jobs.is_empty() => match done {
                Some(key) => { cancellations.remove(&key); }
                None => sampling = false,
            },
            control = subscription.next() => match control {
                Control::Cancel(id) => {
                    if let Some(cancel) = cancellations.remove(&id) {
                        let _: Result<(), ()> = oneshot::Sender::send(cancel, ());
                    }
                }
                Control::Task(task) => {
                    if jobs.len() >= 9 {
                        let _ = task.response.send(TaskReply::failure("busy","节点 SSH 查询繁忙"));
                        continue;
                    }
                    let key = task.request.request_id.clone();
                    let (cancel, mut cancelled) = oneshot::channel::<()>();
                    cancellations.insert(key.clone(), cancel);
                    let target = target.clone();
                    let terminals = terminals.clone();
                    let slots = slots.clone();
                    let pool = pool.clone();
                    let enabled = bus.terminal_enabled();
                    jobs.spawn(async move {
                        let Dispatch { request, response, progress } = task;
                        let seconds = request.expires_at.saturating_sub(unix_time().unwrap_or(i64::MAX)).max(0) as u64;
                        let action = async {
                            if seconds == 0 { return TaskReply::failure("timeout", "请求已过期"); }
                            if request.kind.starts_with("terminal_") {
                                return terminal(&target, &terminals, enabled, request).await;
                            }
                            let Ok(_permit) = slots.try_acquire() else { return TaskReply::failure("busy", "SSH 查询繁忙"); };
                            if request.kind=="operation_execute" {
                                let running = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM operation_jobs WHERE id=? AND state='running' AND cancel_requested_at IS NULL)")
                                    .bind(&request.request_id).fetch_one(&pool).await;
                                if !matches!(running, Ok(true)) { return TaskReply::failure("cancelled", "任务已停止"); }
                            }
                            transport::call(&target, &request.kind, request.args, seconds, Some(&progress)).await
                        };
                        let reply = tokio::select! {
                            result = tokio::time::timeout(Duration::from_secs(seconds), action) =>
                                result.unwrap_or_else(|_| TaskReply::failure("timeout", "SSH 请求超时")),
                            _ = &mut cancelled => TaskReply::failure("cancelled", "SSH 请求已取消"),
                        };
                        let _ = response.send(reply);
                        Some(key)
                    });
                }
            }
        }
    }
}

async fn terminal(
    target: &Target,
    terminals: &Mutex<pty::Terminals>,
    enabled: bool,
    request: asterion_protocol::TaskRequest,
) -> TaskReply {
    let container_enabled = target.policy()["managed"]["containers"]
        .as_object()
        .is_some_and(|items| items.values().any(|spec| spec["terminal"] == true));
    if request.kind == "terminal_container_capability"
        || (request.kind == "terminal_capability" && !enabled)
    {
        return TaskReply::Success {
            value: json!({"enabled":enabled && container_enabled}),
        };
    }
    if !enabled {
        return TaskReply::failure("terminal_disabled", "中心未开启终端");
    }
    if request.kind == "terminal_capability" {
        return transport::call(target, "terminal_capability", json!({}), 8, None).await;
    }
    let mut terminals = terminals.lock().await;
    let container = if request.kind == "terminal_container_open" {
        match transport::call(
            target,
            "container_console_target",
            request.args.clone(),
            8,
            None,
        )
        .await
        {
            TaskReply::Success {
                value: Value::String(id),
            } if asterion_protocol::is_lower_hex(&id, 64) => Some(id),
            failure @ TaskReply::Failure { .. } => return failure,
            _ => return TaskReply::failure("operation_denied", "容器身份无效"),
        }
    } else {
        None
    };
    terminals
        .handle(request, target, container.as_deref())
        .await
}

async fn sample(pool: &SqlitePool, id: &str, target: &Target, period: u64) {
    let response = transport::call(target, "sample", json!({}), 10, None).await;
    if let Err(error) = save_sample(pool, id, period, response).await {
        tracing::warn!(%error,node_id=id,"无法保存 SSH 采样");
    }
}

async fn save_sample(
    pool: &SqlitePool,
    id: &str,
    period: u64,
    reply: TaskReply,
) -> Result<(), sqlx::Error> {
    if let TaskReply::Success { value } = &reply
        && crate::monitor::valid_sample(&value["sample"])
    {
        let mut tx = pool.begin().await?;
        let updated = sqlx::query("UPDATE nodes SET last_seen_at=unixepoch(), ssh_online_until=unixepoch()+?, ssh_error=NULL, os=?, arch=?, wsl_distro=CASE WHEN kind='wsl' THEN ? END WHERE id=?")
            .bind((period * 2 + 15) as i64).bind(value["os"].as_str()).bind(value["arch"].as_str())
            .bind(value["wsl_distro"].as_str()).bind(id).execute(&mut *tx).await?;
        if updated.rows_affected() > 0 {
            crate::monitor::store_metrics(&mut tx, id, &value["sample"]).await?;
        }
        return tx.commit().await;
    }
    let message = match &reply {
        TaskReply::Failure { code, .. } if code == "offline" => {
            "SSH 连接失败，请核对地址、主机密钥和账号密钥"
        }
        TaskReply::Failure { code, .. } if code == "timeout" => "SSH 采样超时",
        _ => "SSH 采样失败，请检查节点 Python 3 与 /proc 读取权限",
    };
    sqlx::query("UPDATE nodes SET ssh_online_until=MIN(COALESCE(ssh_online_until,unixepoch()),unixepoch()), ssh_error=? WHERE id=?")
        .bind(message).bind(id).execute(pool).await?;
    Ok(())
}
