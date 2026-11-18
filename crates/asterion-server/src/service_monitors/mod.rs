mod polling;
mod storage;

#[cfg(test)]
mod tests;

use asterion_protocol::{is_node_id, operation::valid_service_unit};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::auth::{AdminSession, Csrf};
use crate::monitor::{ApiError, ensure_node};

pub use polling::poll_due;
pub use storage::purge;

const NOT_FOUND: ApiError = ApiError::new(
    StatusCode::NOT_FOUND,
    "service_monitor_not_found",
    "找不到该服务的监控记录",
);

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/nodes/{id}/service-monitors", get(list))
        .route(
            "/api/v1/nodes/{id}/service-monitors/{unit}",
            axum::routing::put(enable).delete(disable),
        )
        .route(
            "/api/v1/nodes/{id}/service-monitors/{unit}/history",
            get(history),
        )
}

#[derive(Serialize)]
struct Monitors {
    monitors: Vec<storage::Monitor>,
}

#[derive(Serialize)]
struct History {
    history: Vec<storage::HistoryEntry>,
}

async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
) -> Result<Json<Monitors>, ApiError> {
    ensure_node(&pool, &node_id).await?;
    Ok(Json(Monitors {
        monitors: storage::list(&pool, &node_id).await?,
    }))
}

async fn enable(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path((node_id, unit)): Path<(String, String)>,
) -> Result<Json<storage::Monitor>, ApiError> {
    validate_target(&node_id, &unit)?;
    Ok(Json(storage::enable(&pool, &node_id, &unit).await?))
}

async fn disable(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path((node_id, unit)): Path<(String, String)>,
) -> Result<Json<storage::Monitor>, ApiError> {
    validate_target(&node_id, &unit)?;
    Ok(Json(storage::disable(&pool, &node_id, &unit).await?))
}

async fn history(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path((node_id, unit)): Path<(String, String)>,
) -> Result<Json<History>, ApiError> {
    validate_target(&node_id, &unit)?;
    ensure_node(&pool, &node_id).await?;
    Ok(Json(History {
        history: storage::history(&pool, &node_id, &unit).await?,
    }))
}

fn validate_target(node_id: &str, unit: &str) -> Result<(), ApiError> {
    if !is_node_id(node_id) || !valid_service_unit(unit) {
        return Err(ApiError::INVALID);
    }
    Ok(())
}
