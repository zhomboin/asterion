use std::sync::Arc;

use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::agent_bus::AgentBus;
use crate::auth::AdminSession;

pub fn routes() -> Router<SqlitePool> {
    Router::new().route("/api/v1/settings", get(current))
}

#[derive(Serialize)]
struct Settings {
    sample_interval_secs: u64,
    metric_retention_days: u64,
    trend_retention_days: u64,
    terminal_enabled: bool,
}

async fn current(_: AdminSession, Extension(bus): Extension<Arc<AgentBus>>) -> Json<Settings> {
    let (metric_retention_days, trend_retention_days) = bus.retention_days();
    Json(Settings {
        sample_interval_secs: bus.sample_interval_secs(),
        metric_retention_days,
        trend_retention_days,
        terminal_enabled: bus.terminal_enabled(),
    })
}
