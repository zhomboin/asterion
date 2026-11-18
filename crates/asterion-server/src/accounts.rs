use std::sync::Arc;

use asterion_protocol::operation::valid_username;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::agent_bus::AgentBus;
use crate::auth::AdminSession;
use crate::monitor::{ApiError, bus_error, ensure_node};

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/nodes/{node}/accounts", get(accounts))
        .route("/api/v1/nodes/{node}/accounts/{name}/keys", get(keys))
}

async fn accounts(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, ApiError> {
    ensure_node(&pool, &node).await?;
    bus.query(&node, "accounts", json!({}))
        .await
        .map(Json)
        .map_err(bus_error)
}

async fn keys(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((node, username)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !valid_username(&username) {
        return Err(ApiError::INVALID);
    }
    ensure_node(&pool, &node).await?;
    bus.query(&node, "account_keys", json!({"username":username}))
        .await
        .map(Json)
        .map_err(bus_error)
}
