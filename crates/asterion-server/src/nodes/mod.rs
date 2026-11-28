mod api;
#[cfg(test)]
mod api_tests;
mod credentials;
mod error;
mod hosts;
mod model;
mod ssh_auth;

use axum::{Router, routing::get};
use sqlx::SqlitePool;

pub fn routes() -> Router<SqlitePool> {
    let routes = Router::new()
        .route("/api/v1/nodes", get(api::list).post(api::create))
        .route(
            "/api/v1/nodes/{id}",
            get(api::get).patch(api::update).delete(api::remove),
        );
    #[cfg(feature = "openssh")]
    let routes = routes
        .route("/api/v1/ssh/profiles", get(crate::ssh::profiles))
        .route(
            "/api/v1/ssh/host-key",
            axum::routing::post(crate::ssh::host_key::check),
        );
    routes
}
