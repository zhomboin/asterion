mod error;
mod run;
mod storage;
mod workflow;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use sqlx::SqlitePool;

pub use storage::recover;

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route(
            "/api/v1/nodes/{id}/operation-catalog",
            get(workflow::catalog),
        )
        .route("/api/v1/nodes/{id}/operations", post(workflow::prepare))
        .route("/api/v1/nodes/{id}/managed/{group}", get(workflow::managed))
        .route("/api/v1/operations", get(storage::list))
        .route("/api/v1/operations/{id}", get(storage::get))
        .route("/api/v1/operations/{id}/confirm", post(workflow::confirm))
        .route("/api/v1/operations/{id}/cancel", post(workflow::cancel))
        .layer(DefaultBodyLimit::max(52 * 1024))
}
