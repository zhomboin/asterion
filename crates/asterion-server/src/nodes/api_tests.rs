use asterion_protocol::{is_node_id, is_token};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::sync::watch;
use tower::ServiceExt;

use super::credentials;
use crate::{agent_bus::AgentBus, app, database};

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    csrf: bool,
    body: Value,
) -> Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if csrf {
        builder = builder.header("x-asterion-csrf", "1");
    }
    router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn response_json(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    let body = to_bytes(response.into_body(), 8192).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn admin_cookie(pool: &SqlitePool, router: &Router) -> String {
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString};

    let username = credentials::random_hex::<8>().unwrap();
    let password = credentials::random_hex::<16>().unwrap();
    let salt = SaltString::encode_b64(&[42; 16]).unwrap();
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO panel_admin (id, username, password_hash) VALUES (1, ?, ?)")
        .bind(&username)
        .bind(hash)
        .execute(pool)
        .await
        .unwrap();

    let response = request(
        router,
        "POST",
        "/api/v1/auth/login",
        None,
        true,
        json!({"username": username, "password": password}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn node_management_requires_a_session_and_preserves_partial_updates() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
    let router = app::router(
        pool.clone(),
        None,
        shutdown_receiver,
        AgentBus::new(15).into(),
    );

    let unauthenticated = request(&router, "GET", "/api/v1/nodes", None, false, Value::Null).await;
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    let session = admin_cookie(&pool, &router).await;
    let cookie = Some(session.as_str());
    let node = json!({"name": "工作节点", "kind": "wsl", "tags": ["测试"], "notes": "测试节点"});

    let path = "/api/v1/nodes/enrollment-token";
    let csrf_rejected = request(&router, "POST", path, cookie, false, node.clone()).await;
    assert_eq!(csrf_rejected.status(), StatusCode::FORBIDDEN);
    let created = response_json(
        request(&router, "POST", path, cookie, true, node).await,
        StatusCode::CREATED,
    )
    .await;
    let id = created["node_id"].as_str().unwrap();
    let token = created["token"].as_str().unwrap();
    assert!(is_node_id(id));
    assert!(is_token(token));

    let node_path = format!("/api/v1/nodes/{id}");
    let patched = response_json(
        request(
            &router,
            "PATCH",
            &node_path,
            cookie,
            true,
            json!({"notes": "修改后的备注"}),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(patched["name"], "工作节点");
    assert_eq!(patched["kind"], "wsl");
    assert_eq!(patched["tags"], json!(["测试"]));
    assert_eq!(patched["notes"], "修改后的备注");
    assert_eq!(patched["status"], "pending");
    assert!(patched.get("token").is_none());

    sqlx::query(
        "INSERT INTO service_assets (id, node_id, name, management)
         VALUES ('asset', ?, '测试资产', 'other')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let in_use = request(&router, "DELETE", &node_path, cookie, true, Value::Null).await;
    assert_eq!(in_use.status(), StatusCode::CONFLICT);

    sqlx::query("DELETE FROM service_assets WHERE node_id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let removed = request(&router, "DELETE", &node_path, cookie, true, Value::Null).await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let missing = request(&router, "GET", &node_path, cookie, false, Value::Null).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let enrollment_rows =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM node_enrollment_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(enrollment_rows, 0);
    let audit = sqlx::query_scalar::<_, String>(
        "SELECT action FROM node_audit_events WHERE node_id = ? ORDER BY id",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audit, ["node_create", "node_update", "node_remove"]);
}

#[tokio::test]
async fn list_combines_name_tag_and_status_before_pagination() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
    let router = app::router(
        pool.clone(),
        None,
        shutdown_receiver,
        AgentBus::new(15).into(),
    );
    let session = admin_cookie(&pool, &router).await;
    let cookie = Some(session.as_str());

    sqlx::query(
        "INSERT INTO nodes (id, name, kind, tags, last_seen_at, created_at) VALUES
         ('dev-1', 'Development old', 'linux', '[\"Blue\"]', 1, 1),
         ('dev-2', 'Development next', 'linux', '[\"Blue\"]', NULL, 2),
         ('dev-3', 'Development later', 'linux', '[\"Green\"]', NULL, 3)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let page = response_json(
        request(
            &router,
            "GET",
            "/api/v1/nodes?limit=1&q=DEVELOPMENT&tag=Blue&status=pending",
            cookie,
            false,
            Value::Null,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(page["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(page["nodes"][0]["id"], "dev-2");
    assert_eq!(page["has_more"], false);

    let invalid = request(
        &router,
        "GET",
        "/api/v1/nodes?status=unknown",
        cookie,
        false,
        Value::Null,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}
