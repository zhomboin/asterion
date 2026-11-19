use std::path::Path;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::watch;
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    accounts, agent_bus::AgentBus, alerts, assets, auth, http_policy, monitor, nodes, operations,
    service_monitors, settings, terminal, trends,
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn router(
    pool: SqlitePool,
    web_dir: Option<&Path>,
    shutdown: watch::Receiver<bool>,
    bus: Arc<AgentBus>,
) -> Router {
    let terminals = Arc::new(terminal::TerminalHub::new(bus.terminal_enabled()));
    let mut router = Router::new()
        .route("/healthz", get(health))
        .merge(auth::routes())
        .merge(nodes::routes(shutdown))
        .merge(monitor::routes())
        .merge(assets::routes())
        .merge(operations::routes())
        .merge(terminal::routes())
        .merge(accounts::routes())
        .merge(alerts::routes())
        .merge(trends::routes())
        .merge(service_monitors::routes())
        .merge(settings::routes())
        .layer(Extension(terminals))
        .layer(Extension(bus))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(pool);

    if let Some(web_dir) = web_dir {
        let index = ServeFile::new(web_dir.join("index.html"));
        router = router
            .route_service("/", index.clone())
            .route_service("/login", index.clone())
            .route_service("/overview", index.clone())
            .route_service("/nodes", index.clone())
            .route_service("/nodes/{id}", index.clone())
            .route_service("/nodes/{id}/manage", index.clone())
            .route_service("/nodes/{id}/deploy", index.clone())
            .route_service("/assets", index.clone())
            .route_service("/assets/{id}", index.clone())
            .route_service("/operations", index.clone())
            .route_service("/operations/{id}", index.clone())
            .route_service("/alerts", index.clone())
            .route_service("/settings", index)
            .nest_service("/static", ServeDir::new(web_dir.join("static")));
    }

    router.layer(middleware::from_fn(http_policy::response))
}

async fn health(State(pool): State<SqlitePool>) -> impl IntoResponse {
    let probe = sqlx::query("SELECT 1").execute(&pool).await;
    let (status_code, status) = if probe.is_ok() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
    };

    (
        status_code,
        [(header::CACHE_CONTROL, "no-store")],
        Json(HealthResponse { status }),
    )
}
