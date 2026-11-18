mod evaluation;
mod routes;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use sqlx::SqlitePool;

pub use evaluation::evaluate;

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/alerts", get(routes::events))
        .route(
            "/api/v1/alert-rules",
            get(routes::list).post(routes::create),
        )
        .route(
            "/api/v1/alert-rules/{id}",
            axum::routing::patch(routes::update).delete(routes::remove),
        )
        .layer(DefaultBodyLimit::max(2048))
}
