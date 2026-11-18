mod error;
mod metrics;
mod queries;

use axum::Router;
use axum::routing::get;
use sqlx::SqlitePool;

pub(crate) use error::ApiError;
pub(crate) use metrics::ensure_node;
pub use metrics::{purge_metrics, store_metrics};
pub(crate) use queries::bus_error;

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/nodes/{id}/metrics", get(metrics::history))
        .route("/api/v1/nodes/{id}/processes", get(queries::processes))
        .route("/api/v1/nodes/{id}/processes/{pid}", get(queries::process))
        .route("/api/v1/nodes/{id}/docker", get(queries::docker))
        .route("/api/v1/nodes/{id}/docker/containers", get(queries::docker))
        .route(
            "/api/v1/nodes/{id}/docker/containers/{container}/stats",
            get(queries::docker_stats),
        )
        .route(
            "/api/v1/nodes/{id}/docker/containers/{container}/logs",
            get(queries::docker_logs),
        )
        .route("/api/v1/nodes/{id}/systemd", get(queries::systemd))
        .route("/api/v1/nodes/{id}/systemd/services", get(queries::systemd))
        .route(
            "/api/v1/nodes/{id}/systemd/services/{unit}",
            get(queries::systemd_service),
        )
}
