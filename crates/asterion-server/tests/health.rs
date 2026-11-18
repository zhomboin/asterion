use std::sync::Arc;

use asterion_server::{agent_bus::AgentBus, app, database};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn health_reports_the_database_state() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let (_stopping, shutdown) = tokio::sync::watch::channel(false);
    let router = app::router(pool.clone(), None, shutdown, Arc::new(AgentBus::new(15)));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({"status": "ok"})
    );

    pool.close().await;
    let response = router
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!({"status": "unavailable"})
    );
}
