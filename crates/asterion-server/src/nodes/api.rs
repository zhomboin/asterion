use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::{Sqlite, SqlitePool, Transaction};

use super::credentials::{self, IssuedEnrollment, random_hex};
use super::error::ApiError;
use super::model::{self, NodeFilters, NodeInput, NodeList, NodePatch, NodeStatus, NodeView};
use crate::auth::{AdminSession, Csrf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pagination {
    limit: Option<u16>,
    offset: Option<u32>,
    q: Option<String>,
    tag: Option<String>,
    status: Option<NodeStatus>,
}

pub(super) async fn list(
    _session: AdminSession,
    State(pool): State<SqlitePool>,
    query: Result<Query<Pagination>, QueryRejection>,
) -> Result<Json<NodeList>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID_INPUT)?;
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || offset > 100_000 {
        return Err(ApiError::INVALID_INPUT);
    }
    let filters = NodeFilters {
        q: filter_text(query.q, 100)?,
        tag: filter_text(query.tag, 32)?,
        status: query.status,
    };
    Ok(Json(model::list(&pool, limit, offset, &filters).await?))
}

fn filter_text(value: Option<String>, max_chars: usize) -> Result<Option<String>, ApiError> {
    let value = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match value {
        Some(text) if !model::bounded_text(text, max_chars) => Err(ApiError::INVALID_INPUT),
        value => Ok(value.map(str::to_owned)),
    }
}

pub(super) async fn get(
    _session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<NodeView>, ApiError> {
    Ok(Json(model::get(&pool, &id).await?))
}

pub(super) async fn create(
    _csrf: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    body: Result<Json<NodeInput>, JsonRejection>,
) -> Result<(StatusCode, Json<IssuedEnrollment>), ApiError> {
    let Json(input) = body.map_err(ApiError::invalid_json)?;
    let input = input.validate()?;
    let id = random_hex::<16>()?;
    let mut tx = pool.begin().await?;

    sqlx::query("INSERT INTO nodes (id, name, kind, tags, notes) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&input.name)
        .bind(input.kind.as_str())
        .bind(model::tags_json(&input.tags)?)
        .bind(&input.notes)
        .execute(&mut *tx)
        .await?;

    let enrollment = credentials::issue(&mut tx, &id).await?;
    model::audit(&mut tx, &session.username, &id, "node_create").await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(enrollment)))
}

pub(super) async fn update(
    _csrf: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    body: Result<Json<NodePatch>, JsonRejection>,
) -> Result<Json<NodeView>, ApiError> {
    model::check_id(&id)?;
    let Json(input) = body.map_err(ApiError::invalid_json)?;
    let input = input.validate()?;
    let kind = input.kind.map(|kind| kind.as_str());
    let tags = input.tags.as_deref().map(model::tags_json).transpose()?;
    let mut tx = pool.begin().await?;

    let result = sqlx::query(
        "UPDATE nodes SET name = COALESCE(?, name), kind = COALESCE(?, kind),
             tags = COALESCE(?, tags), notes = COALESCE(?, notes),
             wsl_distro = CASE WHEN ? = 'linux' THEN NULL ELSE wsl_distro END,
             updated_at = unixepoch()
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(kind)
    .bind(tags)
    .bind(&input.notes)
    .bind(kind)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    model::audit(&mut tx, &session.username, &id, "node_update").await?;
    tx.commit().await?;
    Ok(Json(model::get(&pool, &id).await?))
}

pub(super) async fn remove(
    _csrf: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    model::check_id(&id)?;
    let mut tx = pool.begin().await?;
    let has_assets = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM service_assets WHERE node_id = ?)",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    if has_assets {
        return Err(ApiError::IN_USE);
    }

    let deleted = sqlx::query("DELETE FROM nodes WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    model::audit(&mut tx, &session.username, &id, "node_remove").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn issue(
    _csrf: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<IssuedEnrollment>), ApiError> {
    model::check_id(&id)?;
    let mut tx = pool.begin().await?;
    require_node(&mut tx, &id).await?;
    let enrollment = credentials::issue(&mut tx, &id).await?;
    model::audit(&mut tx, &session.username, &id, "enrollment_issue").await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(enrollment)))
}

pub(super) async fn revoke(
    _csrf: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    model::check_id(&id)?;
    let mut tx = pool.begin().await?;
    require_node(&mut tx, &id).await?;
    credentials::revoke(&mut tx, &id).await?;
    model::audit(&mut tx, &session.username, &id, "credential_revoke").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn require_node(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), ApiError> {
    let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    if !exists {
        return Err(ApiError::NOT_FOUND);
    }
    Ok(())
}
