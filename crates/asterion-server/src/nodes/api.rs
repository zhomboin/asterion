use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::SqlitePool;

use super::credentials::random_hex;
use super::error::ApiError;
use super::hosts;
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
    metrics: Option<bool>,
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
        with_metrics: query.metrics.unwrap_or_default(),
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
    local: Option<axum::Extension<crate::local::LocalPanel>>,
    State(pool): State<SqlitePool>,
    body: Result<Json<NodeInput>, JsonRejection>,
) -> Result<(StatusCode, Json<NodeView>), ApiError> {
    let Json(input) = body.map_err(ApiError::invalid_json)?;
    let id = random_hex::<16>()?;
    let input = input.validate(local.is_some())?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let host_id = match input.host_name.as_deref() {
        Some(name) => Some(hosts::resolve_id(&mut tx, input.kind.as_str(), name).await?),
        None => None,
    };

    sqlx::query(
        "INSERT INTO nodes (id, host_id, name, kind, tags, notes, ssh_profile) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(host_id)
    .bind(&input.name)
    .bind(input.kind.as_str())
    .bind(model::tags_json(&input.tags)?)
    .bind(&input.notes)
    .bind(&input.ssh_profile)
    .execute(&mut *tx)
    .await?;

    model::audit(&mut tx, &session.username, &id, "node_create").await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(model::get(&pool, &id).await?)))
}

pub(super) async fn update(
    _csrf: Csrf,
    session: AdminSession,
    local: Option<axum::Extension<crate::local::LocalPanel>>,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    body: Result<Json<NodePatch>, JsonRejection>,
) -> Result<Json<NodeView>, ApiError> {
    model::check_id(&id)?;
    let Json(input) = body.map_err(ApiError::invalid_json)?;
    let input = input.validate(local.is_some())?;
    let kind = input.kind.map(|kind| kind.as_str());
    let tags = input.tags.as_deref().map(model::tags_json).transpose()?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let existing_kind = sqlx::query_scalar::<_, String>("SELECT kind FROM nodes WHERE id = ?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if let Some(profile) = &input.ssh_profile {
        let bound: Option<String> =
            sqlx::query_scalar("SELECT ssh_profile FROM nodes WHERE id = ?")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        if bound.is_some_and(|current| current != *profile) {
            return Err(ApiError::INVALID_INPUT);
        }
    }
    let effective_kind = kind.unwrap_or(&existing_kind);
    let host_id = match input.host_name.as_ref().and_then(|name| name.as_deref()) {
        Some(name) => Some(hosts::resolve_id(&mut tx, effective_kind, name).await?),
        None => None,
    };
    let replace_host = input.host_name.is_some() || effective_kind != existing_kind;

    sqlx::query(
        "UPDATE nodes SET name = COALESCE(?, name), kind = COALESCE(?, kind),
             tags = COALESCE(?, tags), notes = COALESCE(?, notes),
             wsl_distro = CASE WHEN ? = 'linux' THEN NULL ELSE wsl_distro END,
             host_id = CASE WHEN ? THEN ? ELSE host_id END,
             ssh_profile = COALESCE(?, ssh_profile), updated_at = unixepoch()
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(kind)
    .bind(tags)
    .bind(&input.notes)
    .bind(kind)
    .bind(replace_host)
    .bind(host_id)
    .bind(&input.ssh_profile)
    .bind(&id)
    .execute(&mut *tx)
    .await?;

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
