mod discovery;
mod input;
mod records;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use sqlx::SqlitePool;

pub(crate) use discovery::observe;

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/assets", get(records::list).post(records::create))
        .route(
            "/api/v1/assets/{id}",
            get(records::get)
                .patch(records::update)
                .delete(records::remove),
        )
        .route("/api/v1/assets/{id}/bindings", post(records::bind))
        .route(
            "/api/v1/assets/{id}/bindings/{discovery_id}",
            delete(records::unbind),
        )
        .route("/api/v1/nodes/{id}/discoveries", get(discovery::list))
        .layer(DefaultBodyLimit::max(8192))
}
