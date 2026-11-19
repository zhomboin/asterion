mod worker;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::sync::{Semaphore, watch};

use super::{api, credentials, error::ApiError, model::NodeInput};
use crate::auth::{AdminSession, Csrf};
use worker::{Plan, UNIT};

type Owner = [u8; 32];
struct Prepared {
    owner: Owner,
    target: Target,
    plan: Plan,
    at: Instant,
}
impl Prepared {
    fn confirms(&self, owner: &Owner, fingerprint: &str) -> bool {
        self.owner == *owner
            && self.at.elapsed() < Duration::from_secs(120)
            && self.plan.fingerprint == fingerprint
    }
}
struct Job {
    owner: Owner,
    node: String,
    state: &'static str,
    lines: Vec<String>,
    at: Instant,
}
struct Hub {
    plans: Mutex<HashMap<String, Prepared>>,
    jobs: Mutex<HashMap<String, Job>>,
    slots: Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
}

pub(super) fn routes(shutdown: watch::Receiver<bool>) -> Router<SqlitePool> {
    let hub = Hub {
        plans: Mutex::default(),
        jobs: Mutex::default(),
        slots: Arc::new(Semaphore::new(2)),
        shutdown,
    };
    Router::new()
        .route("/api/v1/nodes/ssh/capability", get(capability))
        .route("/api/v1/nodes/ssh/inspect", post(inspect))
        .route("/api/v1/nodes/ssh/deploy", post(start))
        .route("/api/v1/nodes/ssh/deploy/{id}", get(status))
        .layer(DefaultBodyLimit::max(48 * 1024))
        .layer(Extension(Arc::new(hub)))
}

fn available() -> Result<(), ApiError> {
    if worker::enabled() {
        Ok(())
    } else {
        Err(ApiError::deployment(
            StatusCode::SERVICE_UNAVAILABLE,
            "中心未启用 SSH 接入，可选择手动接入",
        ))
    }
}

async fn capability(_: AdminSession) -> Json<Value> {
    Json(json!({"enabled":worker::enabled()}))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    host: String,
    port: u16,
    user: String,
}
impl Target {
    fn valid(&self) -> bool {
        let dns = !self.host.is_empty()
            && self.host.len() <= 253
            && self.host.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
        (dns || self.host.parse::<IpAddr>().is_ok())
            && self.port != 0
            && !self.user.is_empty()
            && self.user.len() <= 64
            && self
                .user
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    }
}

async fn inspect(
    _: Csrf,
    session: AdminSession,
    Extension(hub): Extension<Arc<Hub>>,
    Json(target): Json<Target>,
) -> Result<Json<Value>, ApiError> {
    available()?;
    if !target.valid() {
        return Err(ApiError::INVALID_INPUT);
    }
    let _permit = hub.slots.try_acquire().map_err(|_| ApiError::BUSY)?;
    let mut plan = None;
    let result = worker::call(json!({"mode":"inspect","host":target.host,"port":target.port,"user":target.user,"unit":UNIT}),
        Duration::from_secs(10), hub.shutdown.clone(), |value| {
            plan = Some(serde_json::from_value::<Plan>(value).map_err(|_| "无法解析部署计划")?); Ok(())
        }).await;
    if let Err(message) = result {
        return Ok(Json(json!({"error":message})));
    }
    let plan = plan.ok_or(ApiError::INTERNAL)?;
    let id = credentials::random_hex::<16>()?;
    let response = json!({"ticket":id,"address":plan.address,"fingerprint":plan.fingerprint,"center":plan.center,"commands":plan.commands});
    let mut plans = hub.plans.lock().unwrap();
    plans.retain(|_, value| value.at.elapsed() < Duration::from_secs(120));
    if plans.len() >= 32 {
        return Err(ApiError::BUSY);
    }
    plans.insert(
        id,
        Prepared {
            owner: *session.binding(),
            target,
            plan,
            at: Instant::now(),
        },
    );
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    ticket: String,
    fingerprint: String,
    node: NodeInput,
    method: String,
    secret: String,
    passphrase: Option<String>,
}

async fn start(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(hub): Extension<Arc<Hub>>,
    Json(start): Json<Start>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    available()?;
    let node = start.node.validate()?;
    if start.secret.is_empty()
        || start.secret.len() > 32768
        || !matches!(start.method.as_str(), "password" | "key")
        || start
            .passphrase
            .as_ref()
            .is_some_and(|s| s.len() > 1024 || start.method != "key")
    {
        return Err(ApiError::INVALID_INPUT);
    }
    let permit = hub
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::BUSY)?;
    let prepared = {
        let mut plans = hub.plans.lock().unwrap();
        if !plans
            .get(&start.ticket)
            .is_some_and(|p| p.confirms(session.binding(), &start.fingerprint))
        {
            return Err(ApiError::deployment(
                StatusCode::CONFLICT,
                "指纹确认不匹配或已失效，请重新检查",
            ));
        }
        plans.remove(&start.ticket).unwrap()
    };
    let id = credentials::random_hex::<16>()?;
    {
        let mut jobs = hub.jobs.lock().unwrap();
        jobs.retain(|_, job| job.at.elapsed() < Duration::from_secs(1800));
        if jobs.len() >= 64 {
            return Err(ApiError::BUSY);
        }
    }
    let enrollment = api::register(&pool, &session.username, node).await?;
    let target = prepared.target;
    tracing::info!(action="ssh_agent_deploy", actor=%session.username, node_id=%enrollment.node_id, job_id=%id, "已确认首次 SSH 部署");
    let payload = json!({"mode":"deploy","host":target.host,"port":target.port,"user":target.user,
        "address":prepared.plan.address, "fingerprint":prepared.plan.fingerprint,"manifest":prepared.plan.manifest,
        "method":start.method,"secret":start.secret,"passphrase":start.passphrase,"unit":UNIT,"token":enrollment.token});
    let response = json!({"id":id,"node_id":enrollment.node_id});
    hub.jobs.lock().unwrap().insert(
        id.clone(),
        Job {
            owner: *session.binding(),
            node: enrollment.node_id.clone(),
            state: "running",
            lines: Vec::new(),
            at: Instant::now(),
        },
    );
    tokio::spawn(async move {
        let _permit = permit;
        let result = worker::call(
            payload,
            Duration::from_secs(180),
            hub.shutdown.clone(),
            |value| {
                let line = value["line"].as_str().ok_or("日志格式错误")?;
                let mut jobs = hub.jobs.lock().unwrap();
                let job = jobs.get_mut(&id).ok_or("部署记录已失效")?;
                if job.lines.len() >= 128 || line.len() > 4096 {
                    return Err("部署日志超限".into());
                }
                job.lines.push(line.to_owned());
                Ok(())
            },
        )
        .await;
        if result.is_err() {
            let _ = sqlx::query("UPDATE node_enrollment_tokens SET revoked_at=unixepoch() WHERE node_id=? AND consumed_at IS NULL AND fingerprint=?")
                .bind(&enrollment.node_id).bind({ use sha2::{Digest,Sha256}; Sha256::digest(enrollment.token.as_bytes()).to_vec() })
                .execute(&pool).await;
        }
        if let Some(job) = hub.jobs.lock().unwrap().get_mut(&id) {
            match result {
                Ok(()) => job.state = "deployed",
                Err(message) => {
                    job.lines.push(message);
                    job.state = "failed";
                }
            }
            tracing::info!(action="ssh_agent_deploy_finished", node_id=%job.node, job_id=%id, state=%job.state, "SSH 部署结束");
        }
    });
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[cfg(test)]
mod tests;

async fn status(
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(hub): Extension<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (node, state, lines) = {
        let jobs = hub.jobs.lock().unwrap();
        let job = jobs
            .get(&id)
            .filter(|job| {
                job.owner == *session.binding() && job.at.elapsed() < Duration::from_secs(1800)
            })
            .ok_or(ApiError::NOT_FOUND)?;
        (job.node.clone(), job.state, job.lines.clone())
    };
    let heartbeat = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM nodes n JOIN node_enrollment_tokens t ON t.node_id=n.id JOIN agent_credentials a ON a.node_id=n.id AND a.revoked_at IS NULL WHERE n.id=? AND n.last_seen_at > t.consumed_at AND n.last_seen_at > unixepoch()-?)")
        .bind(&node).bind(asterion_protocol::HEARTBEAT_TIMEOUT_SECS as i64).fetch_one(&pool).await?;
    Ok(Json(
        json!({"state":state, "lines":lines, "heartbeat":heartbeat, "node_id":node}),
    ))
}
