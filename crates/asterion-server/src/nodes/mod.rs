mod api;
mod credentials;
mod error;
mod hosts;
mod model;
mod socket;
mod stream;

#[cfg(test)]
mod api_tests;

#[cfg(test)]
mod tests;

use asterion_protocol::AGENT_CONNECT_PATH;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::{Extension, Router};
use sqlx::SqlitePool;
use tokio::sync::watch;

pub fn routes(shutdown: watch::Receiver<bool>) -> Router<SqlitePool> {
    let agent = Router::new()
        .route(AGENT_CONNECT_PATH, get(socket::connect))
        .layer(Extension(shutdown));

    Router::new()
        .route("/api/v1/nodes", get(api::list))
        .route("/api/v1/nodes/enrollment-token", post(api::create))
        .route(
            "/api/v1/nodes/{id}",
            get(api::get).patch(api::update).delete(api::remove),
        )
        .route("/api/v1/nodes/{id}/enrollment-token", post(api::issue))
        .route("/api/v1/nodes/{id}/credentials", delete(api::revoke))
        .layer(DefaultBodyLimit::max(8192))
        .merge(agent)
}
