use asterion_protocol::is_node_id;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use super::error::NOT_FOUND;
use crate::auth::AdminSession;
use crate::monitor::ApiError;

const SUMMARY_QUERY: &str = "SELECT id, node_id, node_name, actor, action, step_up_required,
    CASE WHEN state = 'prepared' AND expires_at <= unixepoch() THEN 'expired' ELSE state END AS state,
    created_at, expires_at, started_at, finished_at, cancel_requested_at, exit_code,
    duration_ms, error_code FROM operation_jobs";

#[derive(FromRow, Serialize)]
pub(super) struct Summary {
    id: String,
    node_id: String,
    node_name: String,
    actor: String,
    #[serde(serialize_with = "action_json")]
    action: String,
    step_up_required: bool,
    state: String,
    created_at: i64,
    expires_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    cancel_requested_at: Option<i64>,
    exit_code: Option<i64>,
    duration_ms: Option<i64>,
    error_code: Option<String>,
}

fn action_json<S: Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serde_json::from_str::<Value>(text)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

#[derive(Serialize)]
pub(super) struct List {
    operations: Vec<Summary>,
    has_more: bool,
}

#[derive(Serialize)]
pub(super) struct Detail {
    #[serde(flatten)]
    summary: Summary,
    can_cancel: bool,
    output: Option<String>,
    truncated: bool,
    audit: Vec<AuditEntry>,
}

#[derive(FromRow, Serialize)]
struct AuditEntry {
    actor: Option<String>,
    action: String,
    observed_at: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Filter {
    node_id: Option<String>,
    limit: Option<u16>,
    offset: Option<u32>,
}

pub(super) async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    options: Result<Query<Filter>, QueryRejection>,
) -> Result<Json<List>, ApiError> {
    let Query(query) = options.map_err(|_| ApiError::INVALID)?;
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    let id = query.node_id.as_deref();
    if !(1..=100).contains(&limit) || offset > 100_000 || id.is_some_and(|id| !is_node_id(id)) {
        return Err(ApiError::INVALID);
    }
    let query = format!(
        "{SUMMARY_QUERY} WHERE (? IS NULL OR node_id = ?) ORDER BY created_at DESC, id LIMIT ? OFFSET ?"
    );
    let mut operations = sqlx::query_as::<_, Summary>(&query)
        .bind(id)
        .bind(id)
        .bind(i64::from(limit) + 1)
        .bind(offset)
        .fetch_all(&pool)
        .await?;
    let has_more = operations.len() > usize::from(limit);
    operations.truncate(usize::from(limit));
    Ok(Json(List {
        operations,
        has_more,
    }))
}

pub(super) async fn get(
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<Detail>, ApiError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND);
    }

    let mut tx = pool.begin().await?;
    let query = format!("{SUMMARY_QUERY} WHERE id = ?");
    let summary = sqlx::query_as::<_, Summary>(&query)
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(NOT_FOUND)?;
    let (output, truncated, same_session) = sqlx::query_as::<_, (Option<String>, bool, bool)>(
        "SELECT output, truncated, actor = ? AND session_hash = ? FROM operation_jobs WHERE id = ?",
    )
    .bind(&session.username)
    .bind(session.binding().as_slice())
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let audit = sqlx::query_as::<_, AuditEntry>(
        "SELECT actor, action, observed_at FROM operation_audit_events WHERE task_id = ? ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let can_cancel = same_session
        && matches!(summary.state.as_str(), "prepared" | "queued" | "running")
        && summary.cancel_requested_at.is_none();
    Ok(Json(Detail {
        summary,
        can_cancel,
        output,
        truncated,
        audit,
    }))
}

pub(super) async fn audit(
    connection: &mut SqliteConnection,
    task_id: &str,
    actor: Option<&str>,
    action: &'static str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO operation_audit_events (task_id, actor, action) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(actor)
        .bind(action)
        .execute(connection)
        .await?;
    Ok(())
}

pub(super) async fn expire(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE operation_jobs SET state = 'expired', challenge_hash = NULL, finished_at = unixepoch()
         WHERE state = 'prepared' AND expires_at <= unixepoch()",
    )
    .execute(connection)
    .await?;
    Ok(())
}

pub async fn recover(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    expire(&mut tx).await?;
    let ids = sqlx::query_scalar::<_, String>(
        "UPDATE operation_jobs SET state = 'interrupted', error_code = 'center_restarted', finished_at = unixepoch()
         WHERE state IN ('queued', 'running') RETURNING id",
    )
    .fetch_all(&mut *tx)
    .await?;
    for id in ids {
        audit(&mut tx, &id, None, "interrupted").await?;
    }
    tx.commit().await?;
    Ok(())
}
