use std::sync::Arc;

use asterion_server::{agent_bus::AgentBus, alerts, assets, database, monitor, operations, trends};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tower::ServiceExt;

async fn fixture() -> (SqlitePool, Router, String, String) {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let router = Router::new()
        .merge(assets::routes())
        .merge(monitor::routes())
        .merge(operations::routes())
        .merge(alerts::routes())
        .merge(trends::routes())
        .with_state(pool.clone())
        .layer(axum::Extension(Arc::new(AgentBus::new(15))));
    let node = new_id();
    sqlx::query("INSERT INTO panel_admin (id, username, password_hash) VALUES (1, 'test', 'unused-in-test')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO nodes (id, name, kind) VALUES (?, '测试节点', 'linux')")
        .bind(&node)
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session(&pool).await;
    (pool, router, cookie, node)
}

fn new_id() -> String {
    entropy::<16>()
}

fn entropy<const LENGTH: usize>() -> String {
    let mut bytes = [0_u8; LENGTH];
    getrandom::fill(&mut bytes).unwrap();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn session(pool: &SqlitePool) -> String {
    let token = entropy::<32>();
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    sqlx::query(
        "INSERT INTO panel_sessions (token_hash, admin_id, expires_at)
         VALUES (?, 1, unixepoch() + 600)",
    )
    .bind(digest.as_slice())
    .execute(pool)
    .await
    .unwrap();
    format!("__Host-asterion_session={token}")
}

async fn call(
    router: &Router,
    cookie: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-asterion-csrf", "1")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (status, body)
}

#[tokio::test]
async fn asset_binding_is_scoped_to_its_node_and_keeps_manual_fields() {
    let (pool, router, cookie, node) = fixture().await;
    let other = new_id();
    sqlx::query("INSERT INTO nodes (id, name, kind) VALUES (?, '其他节点', 'linux')")
        .bind(&other)
        .execute(&pool)
        .await
        .unwrap();

    let (status, asset) = call(
        &router,
        &cookie,
        "POST",
        "/api/v1/assets",
        json!({
            "node_id":node, "name":"服务目录", "management":"systemd",
            "repo_url":"https://example.invalid/repository", "deploy_path":"/example/service",
            "tags":["测试"], "verified":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let asset_id = asset["id"].as_str().unwrap();
    assert_eq!(asset["source"], "manual");
    let path = format!("/api/v1/assets/{asset_id}");
    let binding_path = format!("{path}/bindings");

    let valid_discovery = new_id();
    let foreign_discovery = new_id();
    for (id, owner) in [(&valid_discovery, &node), (&foreign_discovery, &other)] {
        sqlx::query(
            "INSERT INTO discovered_resources
                (id,node_id,kind,resource_scope,resource_id,name,metadata,observed_at)
             VALUES (?,?,'systemd','systemd','test.service','测试服务',
                '{\"working_directory\":\"/example/observed\"}', unixepoch())",
        )
        .bind(id)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (status, _) = call(
        &router,
        &cookie,
        "POST",
        &binding_path,
        json!({"discovery_id":foreign_discovery}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, updated) = call(
        &router,
        &cookie,
        "POST",
        &binding_path,
        json!({"discovery_id":valid_discovery}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["deploy_path"], "/example/service");
    assert_eq!(updated["bindings"][0]["source"], "agent");
    assert_eq!(
        updated["bindings"][0]["metadata"]["working_directory"],
        "/example/observed"
    );
    assert!(updated["last_seen_at"].is_number());

    let (status, _) = call(&router, &cookie, "PATCH", &path, json!({"node_id":other})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, updated) = call(&router, &cookie, "PATCH", &path, json!({"repo_url":null})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(updated["repo_url"].is_null());
    assert!(updated["verified_at"].is_null());
    assert_eq!(updated["tags"], json!(["测试"]));

    let (_, results) = call(
        &router,
        &cookie,
        "GET",
        "/api/v1/assets?q=%E6%9C%8D&management=systemd&tag=%E6%B5%8B%E8%AF%95",
        Value::Null,
    )
    .await;
    assert_eq!(results["assets"][0]["id"], asset_id);

    let (status, _) = call(&router, &cookie, "DELETE", &path, Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM asset_bindings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn operation_confirmation_is_single_use_and_bound_to_the_panel_session() {
    let (pool, router, cookie, node) = fixture().await;
    let other_cookie = session(&pool).await;
    let path = format!("/api/v1/nodes/{node}/operations");
    let (status, created) = call(
        &router,
        &cookie,
        "POST",
        &path,
        json!({
            "type":"command", "name":"fixture", "parameters":{}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["task_id"].as_str().unwrap();
    let challenge = created["challenge"].as_str().unwrap();
    let task_path = format!("/api/v1/operations/{id}");
    let confirm = format!("{task_path}/confirm");

    let denied = call(
        &router,
        &other_cookie,
        "POST",
        &confirm,
        json!({"challenge":challenge}),
    )
    .await;
    assert_eq!(denied.0, StatusCode::CONFLICT);
    let allowed = call(
        &router,
        &cookie,
        "POST",
        &confirm,
        json!({"challenge":challenge}),
    )
    .await;
    assert_eq!(allowed.0, StatusCode::ACCEPTED);
    let replay = call(
        &router,
        &cookie,
        "POST",
        &confirm,
        json!({"challenge":challenge}),
    )
    .await;
    assert_eq!(replay.0, StatusCode::CONFLICT);

    let finished = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let (_, job) = call(&router, &cookie, "GET", &task_path, Value::Null).await;
            assert!(job.get("challenge").is_none());
            if job["state"] == "disconnected" {
                break job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(finished["error_code"], "agent_offline");
    assert!(finished["started_at"].is_null());
    assert!(
        !finished["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["action"] == "dispatched")
    );
    assert_eq!(finished["audit"][1]["action"], "confirmed");
    assert_eq!(
        finished["audit"].as_array().unwrap().last().unwrap()["action"],
        "disconnected"
    );

    let (_, elevated) = call(
        &router,
        &cookie,
        "POST",
        &path,
        json!({
            "type":"process", "pid":200, "start_ticks":1234, "signal":"kill"
        }),
    )
    .await;
    assert_eq!(elevated["step_up_required"], true);
    let task_id = elevated["task_id"].as_str().unwrap();
    let (status, error) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/v1/operations/{task_id}/confirm"),
        json!({"challenge":elevated["challenge"]}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["code"], "step_up_required");
    let (status, _) = call(
        &router,
        &cookie,
        "POST",
        &format!("/api/v1/operations/{task_id}/cancel"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (_, cancelled) = call(
        &router,
        &cookie,
        "GET",
        &format!("/api/v1/operations/{task_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(cancelled["state"], "cancelled");

    operations::recover(&pool).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM operation_jobs WHERE challenge_hash IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

fn metric(cpu_percent: u16) -> Value {
    json!({
        "captured_at":0, "cpu_percent":cpu_percent, "logical_cores":2, "load":[0, 0, 0],
        "memory":{"total_bytes":100,"available_bytes":70,"used_bytes":30},
        "swap":{"total_bytes":0,"available_bytes":0,"used_bytes":0},
        "filesystems":[], "interfaces":[]
    })
}

#[tokio::test]
async fn threshold_alerts_recover_and_rollups_survive_raw_retention() {
    let (pool, router, cookie, node) = fixture().await;
    {
        let mut connection = pool.acquire().await.unwrap();
        monitor::store_metrics(&mut connection, &node, &metric(90))
            .await
            .unwrap();
        sqlx::query("UPDATE metric_samples SET observed_at = unixepoch() - 15")
            .execute(&mut *connection)
            .await
            .unwrap();
    }

    let (status, _) = call(
        &router,
        &cookie,
        "POST",
        "/api/v1/alert-rules",
        json!({
            "node_id":node, "metric":"cpu", "threshold":80, "for_seconds":0, "enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    alerts::evaluate(&pool, 15).await.unwrap();
    alerts::evaluate(&pool, 15).await.unwrap();
    let (_, active) = call(&router, &cookie, "GET", "/api/v1/alerts", Value::Null).await;
    assert_eq!(active["alerts"].as_array().unwrap().len(), 1);
    assert_eq!(active["alerts"][0]["value"], 90.0);

    {
        let mut connection = pool.acquire().await.unwrap();
        monitor::store_metrics(&mut connection, &node, &metric(10))
            .await
            .unwrap();
    }
    alerts::evaluate(&pool, 15).await.unwrap();
    let (_, resolved) = call(
        &router,
        &cookie,
        "GET",
        "/api/v1/alerts?state=resolved",
        Value::Null,
    )
    .await;
    assert_eq!(resolved["alerts"].as_array().unwrap().len(), 1);
    assert_eq!(resolved["alerts"][0]["id"], active["alerts"][0]["id"]);

    let history = format!("/api/v1/nodes/{node}/metrics?range=1h&limit=1");
    let (status, latest) = call(&router, &cookie, "GET", &history, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(latest["samples"].as_array().unwrap().len(), 1);
    assert_eq!(latest["samples"][0]["sample"]["cpu_percent"], 10);

    sqlx::query("UPDATE metric_samples SET observed_at = observed_at - 86400")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(monitor::purge_metrics(&pool, 3600).await.unwrap(), 2);
    let trend = format!("/api/v1/nodes/{node}/trends?range=7d");
    let (status, data) = call(&router, &cookie, "GET", &trend, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["bucket_seconds"], 300);
    let buckets = data["samples"].as_array().unwrap();
    let count: i64 = buckets
        .iter()
        .map(|sample| sample["sample_count"].as_i64().unwrap())
        .sum();
    let cpu: f64 = buckets
        .iter()
        .map(|sample| {
            assert!(sample["disk_percent"].is_null());
            sample["cpu_percent"].as_f64().unwrap() * sample["sample_count"].as_f64().unwrap()
        })
        .sum();
    assert_eq!(count, 2);
    assert_eq!(cpu / count as f64, 50.0);
}
