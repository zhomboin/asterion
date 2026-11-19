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
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/nodes/ssh/capability",
            None,
            false,
            Value::Null
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &router,
            "GET",
            "/api/v1/nodes/ssh/capability",
            cookie,
            false,
            Value::Null
        )
        .await
        .status(),
        StatusCode::OK
    );
    for operation in ["inspect", "deploy"] {
        let path = format!("/api/v1/nodes/ssh/{operation}");
        assert_eq!(
            request(&router, "POST", &path, cookie, false, json!({}))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
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

#[tokio::test]
async fn list_includes_latest_metrics_per_node_when_requested() {
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
        "INSERT INTO nodes (id, name, kind, created_at) VALUES
         ('dev-1', 'Development', 'linux', 1),
         ('dev-2', 'Staging', 'wsl', 2),
         ('dev-3', 'Production', 'linux', 3)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (node, when, sample) in [
        ("dev-1", 1, json!({"cpu_percent": 80.0})),
        (
            "dev-1",
            2,
            json!({
                "cpu_percent": 0.0,
                "memory": {"total_bytes": 200, "used_bytes": 50},
                "filesystems": [
                    {"total_bytes": 100, "available_bytes": 100},
                    {"total_bytes": 200, "available_bytes": 50},
                ],
            }),
        ),
        ("dev-1", i64::MAX, json!({"cpu_percent": 95.0})),
        ("dev-2", 3, json!({"cpu_percent": 10.0})),
    ] {
        sqlx::query("INSERT INTO metric_samples (node_id, observed_at, sample) VALUES (?, ?, ?)")
            .bind(node)
            .bind(when)
            .bind(sample.to_string())
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "UPDATE nodes SET last_seen_at = CASE WHEN id = 'dev-1' THEN unixepoch() ELSE 1 END
         WHERE id IN ('dev-1', 'dev-3')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_credentials (fingerprint, node_id, created_at, connection_generation)
         VALUES (zeroblob(32), 'dev-1', unixepoch(), 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let list = |path| request(&router, "GET", path, cookie, false, Value::Null);
    let result = response_json(list("/api/v1/nodes?metrics=true").await, StatusCode::OK).await;
    assert_eq!(
        result["nodes"][0]["metrics"],
        json!({"observed_at": 2, "cpu_percent": 0.0, "memory_percent": 25.0, "disk_percent": 75.0}),
    );
    assert_eq!(
        result["nodes"][1]["metrics"],
        json!({"observed_at": 3, "cpu_percent": 10.0, "memory_percent": null, "disk_percent": null}),
    );
    assert!(result["nodes"][2].get("metrics").is_none());
    let paginated = response_json(
        list("/api/v1/nodes?metrics=true&limit=1&q=staging").await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(paginated["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(
        paginated["status_counts"],
        json!({"pending": 1, "online": 1, "offline": 1})
    );
    let default = response_json(list("/api/v1/nodes").await, StatusCode::OK).await;
    assert!(default["nodes"][0].get("metrics").is_none());
    assert!(default.get("status_counts").is_none());
}

#[tokio::test]
async fn nodes_can_share_preserve_and_clear_host_binding() {
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

    let create = |node, host| {
        request(
            &router,
            "POST",
            "/api/v1/nodes/enrollment-token",
            cookie,
            true,
            json!({"name": node, "kind": "wsl", "host_name": host}),
        )
    };
    let first = response_json(create("Dist 1", " Twin ").await, StatusCode::CREATED).await;
    let second = response_json(create("Dist 2", "Twin").await, StatusCode::CREATED).await;
    let first_path = format!("/api/v1/nodes/{}", first["node_id"].as_str().unwrap());
    let second_path = format!("/api/v1/nodes/{}", second["node_id"].as_str().unwrap());
    let get = |path| request(&router, "GET", path, cookie, false, Value::Null);
    let first = response_json(get(&first_path).await, StatusCode::OK).await;
    let second = response_json(get(&second_path).await, StatusCode::OK).await;
    assert_eq!(first["host_name"], "Twin");
    assert!(first["host_id"].as_str().is_some());
    assert_eq!(first["host_id"], second["host_id"]);
    let host_kind = sqlx::query_scalar::<_, String>("SELECT kind FROM hosts WHERE id = ?")
        .bind(first["host_id"].as_str().unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(host_kind, "windows");

    let patch = |path, body| request(&router, "PATCH", path, cookie, true, body);
    let revised = response_json(
        patch(&first_path, json!({"notes": "one"})).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(revised["host_id"], first["host_id"]);
    assert_eq!(
        patch(&first_path, json!({"host_name": "   "}))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let cleared = response_json(
        patch(&first_path, json!({"host_name": null})).await,
        StatusCode::OK,
    )
    .await;
    assert!(cleared["host_id"].is_null());
    assert!(cleared["host_name"].is_null());
    let moved = response_json(
        patch(&second_path, json!({"kind": "linux"})).await,
        StatusCode::OK,
    )
    .await;
    assert!(moved["host_id"].is_null());
}
