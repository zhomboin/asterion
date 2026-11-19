use std::sync::Arc;

use asterion_protocol::operation::{OperationAction, OperationTask};
use asterion_protocol::{is_node_id, is_token};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Executor, Sqlite, SqlitePool};

use super::error::{ConfirmError, INVALID_CONFIRMATION, LIMIT_REACHED, NOT_FOUND, bus_error};
use super::{run, storage};
use crate::agent_bus::AgentBus;
use crate::auth::{AdminSession, AuthError, Csrf, verify_step_up};
use crate::monitor::ApiError;

const CONFIRMATION_SECS: i64 = 120;
const MAX_PREPARED_PER_SESSION: i64 = 16;
const MAX_IN_FLIGHT: i64 = 8;

#[derive(Deserialize)]
pub(super) struct Selection {
    key: Option<String>,
}

pub(super) async fn managed(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((node, group)): Path<(String, String)>,
    axum::extract::Query(selection): axum::extract::Query<Selection>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(group.as_str(), "configs" | "packages" | "containers")
        || selection
            .key
            .as_deref()
            .is_some_and(|key| !asterion_protocol::operation::valid_identifier(key, 64))
    {
        return Err(ApiError::INVALID);
    }
    node_name(&pool, &node).await?;
    bus.query(
        &node,
        "managed_query",
        json!({"group":group,"key":selection.key}),
    )
    .await
    .map(Json)
    .map_err(bus_error)
}

#[derive(Serialize)]
pub(super) struct Preparation {
    task_id: String,
    challenge: String,
    action: OperationAction,
    expires_at: i64,
    step_up_required: bool,
}

pub(super) async fn catalog(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, ApiError> {
    node_name(&pool, &node).await?;
    let catalog = bus
        .query(&node, "operation_catalog", json!({}))
        .await
        .map_err(bus_error)?;
    Ok(Json(catalog))
}

pub(super) async fn prepare(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(node): Path<String>,
    body: Result<Json<OperationAction>, JsonRejection>,
) -> Result<(StatusCode, Json<Preparation>), ApiError> {
    let Json(action) = body.map_err(ApiError::invalid_json)?;
    if !action.valid() {
        return Err(ApiError::INVALID);
    }
    let id = random::<16>()?;
    let challenge = random::<32>()?;
    let challenge_hash = hash(&challenge);
    let encoded = serde_json::to_string(&action).map_err(sqlx::Error::decode)?;
    let step_up_required = action.privileged();
    let mut tx = pool.begin().await?;
    let node_name = node_name(&mut *tx, &node).await?;
    storage::expire(&mut tx).await?;
    if sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM operation_jobs WHERE state = 'prepared' AND session_hash = ?",
    )
    .bind(session.binding().as_slice())
    .fetch_one(&mut *tx)
    .await?
        >= MAX_PREPARED_PER_SESSION
    {
        return Err(LIMIT_REACHED);
    }
    let expires_at = sqlx::query_scalar(
        "INSERT INTO operation_jobs (id, node_id, node_name, actor, session_hash, challenge_hash,
         action, step_up_required, expires_at, state)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, unixepoch() + ?, 'prepared') RETURNING expires_at",
    )
    .bind(&id)
    .bind(&node)
    .bind(&node_name)
    .bind(&session.username)
    .bind(session.binding().as_slice())
    .bind(challenge_hash.as_slice())
    .bind(encoded)
    .bind(step_up_required)
    .bind(CONFIRMATION_SECS)
    .fetch_one(&mut *tx)
    .await?;
    storage::audit(&mut tx, &id, Some(&session.username), "prepared").await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(Preparation {
            task_id: id,
            challenge,
            action,
            expires_at,
            step_up_required,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Confirmation {
    challenge: String,
    password: Option<String>,
    content: Option<String>,
}

pub(super) async fn confirm(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(id): Path<String>,
    body: Result<Json<Confirmation>, JsonRejection>,
) -> Result<StatusCode, ConfirmError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND.into());
    }
    let Json(body) = body.map_err(ApiError::invalid_json)?;
    if !is_token(&body.challenge) {
        return Err(ApiError::INVALID.into());
    }
    let hash = hash(&body.challenge);
    let step_up_required = sqlx::query_scalar::<_, bool>(
        "SELECT step_up_required FROM operation_jobs WHERE id = ? AND actor = ? AND
         session_hash = ? AND challenge_hash = ? AND state = 'prepared' AND expires_at > unixepoch()",
    )
    .bind(&id)
    .bind(&session.username)
    .bind(session.binding().as_slice())
    .bind(hash.as_slice())
    .fetch_optional(&pool)
    .await
    .map_err(ApiError::from)?
    .ok_or(INVALID_CONFIRMATION)?;

    if step_up_required {
        let password = body.password.as_deref().ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "step_up_required",
                "请重新验证管理员口令",
            )
        })?;
        match verify_step_up(&pool, &session, password).await {
            Ok(()) => {}
            Err(AuthError::InvalidCredentials) => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "invalid_step_up_password",
                    "管理员口令验证失败",
                )
                .into());
            }
            Err(error) => return Err(error.into()),
        }
    }

    // 保证已落库的确认不依赖浏览器持续等待响应。
    tokio::spawn(async move {
        let target = claim(&pool, &session, &id, &hash, body.content.as_deref()).await?;
        tokio::spawn(run::execute(
            pool,
            bus,
            target.node_id,
            OperationTask {
                task_id: id,
                action: target.action,
                content: body.content,
            },
        ));
        Ok::<_, ConfirmError>(())
    })
    .await
    .map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "operation_worker_failed",
            "中心任务处理失败",
        )
    })??;
    Ok(StatusCode::ACCEPTED)
}

struct Target {
    node_id: String,
    action: OperationAction,
}

async fn claim(
    pool: &SqlitePool,
    session: &AdminSession,
    id: &str,
    challenge_hash: &[u8; 32],
    content: Option<&str>,
) -> Result<Target, ConfirmError> {
    let mut tx = pool.begin().await.map_err(ApiError::from)?;
    session.ensure_active(&mut tx).await?;

    let (node_id, encoded) = sqlx::query_as::<_, (String, String)>(
        "SELECT node_id, action FROM operation_jobs WHERE id = ? AND actor = ?
         AND session_hash = ? AND challenge_hash = ? AND state = 'prepared' AND expires_at > unixepoch()",
    )
    .bind(id)
    .bind(&session.username)
    .bind(session.binding().as_slice())
    .bind(challenge_hash.as_slice())
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::from)?
    .ok_or(INVALID_CONFIRMATION)?;
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(&node_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::NODE_NOT_FOUND.into());
    }

    let action: OperationAction = serde_json::from_str(&encoded)
        .map_err(sqlx::Error::decode)
        .map_err(ApiError::from)?;
    if !action.valid() || !content_matches(&action, content) {
        return Err(ApiError::INVALID.into());
    }
    let (in_flight, node_busy) = sqlx::query_as::<_, (i64, bool)>(
        "SELECT count(*), COALESCE(MAX(node_id = ?), 0)
         FROM operation_jobs WHERE state IN ('queued', 'running')",
    )
    .bind(&node_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    if in_flight >= MAX_IN_FLIGHT || node_busy {
        return Err(LIMIT_REACHED.into());
    }

    let updated = sqlx::query(
        "UPDATE operation_jobs SET state = 'queued', challenge_hash = NULL
         WHERE id = ? AND state = 'prepared' AND expires_at > unixepoch()",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    if updated.rows_affected() == 0 {
        return Err(INVALID_CONFIRMATION.into());
    }
    storage::audit(&mut tx, id, Some(&session.username), "confirmed")
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Target { node_id, action })
}

pub(super) async fn cancel(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND);
    }
    let mut tx = pool.begin().await?;
    storage::expire(&mut tx).await?;
    let (node, state, managed) = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT node_id, state, json_extract(action, '$.type') = 'managed' FROM operation_jobs WHERE id = ? AND actor = ? AND session_hash = ?",
    )
    .bind(&id)
    .bind(&session.username)
    .bind(session.binding().as_slice())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(NOT_FOUND)?;

    if state == "running" {
        if managed {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "operation_not_cancellable",
                "节点正在应用变更，不能强制中断",
            ));
        }
        let updated = sqlx::query(
            "UPDATE operation_jobs SET cancel_requested_at = unixepoch()
             WHERE id = ? AND cancel_requested_at IS NULL",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 0 {
            storage::audit(&mut tx, &id, Some(&session.username), "cancel_requested").await?;
        }
        bus.cancel(&node, &id).map_err(bus_error)?;
    } else if matches!(state.as_str(), "prepared" | "queued") {
        sqlx::query(
            "UPDATE operation_jobs SET state = 'cancelled', challenge_hash = NULL,
             finished_at = unixepoch(), cancel_requested_at = unixepoch() WHERE id = ?",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        storage::audit(&mut tx, &id, Some(&session.username), "cancelled").await?;
    }
    tx.commit().await?;
    Ok(StatusCode::ACCEPTED)
}

async fn node_name<'e>(
    executor: impl Executor<'e, Database = Sqlite>,
    node: &str,
) -> Result<String, ApiError> {
    if !is_node_id(node) {
        return Err(ApiError::NODE_NOT_FOUND);
    }
    sqlx::query_scalar("SELECT name FROM nodes WHERE id = ?")
        .bind(node)
        .fetch_optional(executor)
        .await?
        .ok_or(ApiError::NODE_NOT_FOUND)
}

fn hash(challenge: &str) -> [u8; 32] {
    Sha256::digest(challenge.as_bytes()).into()
}

fn content_matches(action: &OperationAction, content: Option<&str>) -> bool {
    match action {
        OperationAction::Managed { change } if change.digest.is_some() => {
            content.is_some_and(|text| {
                text.len() <= 8192
                    && !text.contains('\0')
                    && change.digest.as_deref()
                        == Some(&format!("{:x}", Sha256::digest(text.as_bytes())))
            })
        }
        _ => content.is_none(),
    }
}

fn random<const N: usize>() -> Result<String, ApiError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "operation_entropy_unavailable",
            "中心暂时无法生成确认凭证",
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod managed_tests {
    use super::*;
    use asterion_protocol::managed::{ManagedAction, ManagedVerb};

    #[test]
    fn confirmation_is_bound_to_exact_content_and_does_not_persist_it() {
        let text = "secret fixture";
        let action = OperationAction::Managed {
            change: ManagedAction {
                key: "app".into(),
                verb: ManagedVerb::SaveReload,
                revision: "a".repeat(64),
                digest: Some(format!("{:x}", Sha256::digest(text))),
                profile: None,
                settings: None,
            },
        };
        assert!(action.valid() && action.privileged());
        assert!(content_matches(&action, Some(text)));
        assert!(!content_matches(&action, Some("different")));
        assert!(!content_matches(&action, None));
        assert!(!serde_json::to_string(&action).unwrap().contains(text));
    }
}
