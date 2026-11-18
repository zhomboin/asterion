use asterion_protocol::{AgentMessage, PROTOCOL_VERSION};
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::credentials::{self, IssuedEnrollment};
use super::model;
use super::socket::{SocketFailure, decode, validate_heartbeat, validate_hello};
use crate::database;

const NODE: &str = "1234567890abcdef1234567890abcdef";
const OTHER_NODE: &str = "abcdef1234567890abcdef1234567890";
const FIRST_DEVICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND_DEVICE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn seed_node(pool: &SqlitePool, node: &str) {
    sqlx::query("INSERT INTO nodes (id, name, kind) VALUES (?, ?, 'wsl')")
        .bind(node)
        .bind(node)
        .execute(pool)
        .await
        .unwrap();
}

async fn issue(pool: &SqlitePool, node: &str) -> IssuedEnrollment {
    let mut tx = pool.begin().await.unwrap();
    let issued = credentials::issue(&mut tx, node).await.unwrap();
    tx.commit().await.unwrap();
    issued
}

fn hello() -> Value {
    json!({
        "type": "hello",
        "protocol_version": PROTOCOL_VERSION,
        "node_id": null,
        "message_id": 1,
        "timestamp": 0,
        "payload": {
            "os": "Linux",
            "arch": "x86_64",
            "wsl_distro": "Ubuntu",
            "agent_version": "0.1.0"
        }
    })
}

#[tokio::test]
async fn registration_and_rotation_keep_a_stable_node_identity() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    seed_node(&pool, NODE).await;
    let pending = serde_json::to_value(model::get(&pool, NODE).await.unwrap()).unwrap();
    assert_eq!(pending["status"], "pending");
    assert_eq!(pending["last_seen_at"], Value::Null);

    let issued = issue(&pool, NODE).await;
    let hello = decode(&hello().to_string()).unwrap();
    let metadata = validate_hello(&hello, NODE).unwrap();
    let candidate = credentials::authenticate(&pool, FIRST_DEVICE, Some(&issued.token))
        .await
        .unwrap();
    let second_candidate = credentials::authenticate(&pool, SECOND_DEVICE, Some(&issued.token))
        .await
        .unwrap();
    let (initial_connection, seen_at) = credentials::connect(&pool, candidate, metadata)
        .await
        .unwrap();
    assert_eq!(initial_connection.node_id, NODE);
    assert!(seen_at > 0);
    assert_eq!(
        credentials::connect(&pool, second_candidate, metadata)
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        credentials::authenticate(&pool, SECOND_DEVICE, Some(&issued.token))
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::UNAUTHORIZED
    );

    // Reusing the same bearer is safe after the enrollment response is lost.
    let candidate = credentials::authenticate(&pool, FIRST_DEVICE, Some(&issued.token))
        .await
        .unwrap();
    let (connection, _) = credentials::connect(&pool, candidate, metadata)
        .await
        .unwrap();
    assert_eq!(connection.node_id, NODE);
    assert_eq!(
        credentials::heartbeat(&pool, &initial_connection)
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_ne!(credentials::heartbeat(&pool, &connection).await.unwrap(), 0);

    let online = serde_json::to_value(model::get(&pool, NODE).await.unwrap()).unwrap();
    assert_eq!(online["status"], "online");
    assert_eq!(online["wsl_distro"], "Ubuntu");

    sqlx::query("UPDATE nodes SET last_seen_at = unixepoch() - 30 WHERE id = ?")
        .bind(NODE)
        .execute(&pool)
        .await
        .unwrap();
    let expired = serde_json::to_value(model::get(&pool, NODE).await.unwrap()).unwrap();
    assert_eq!(expired["status"], "offline");
    assert!(expired["last_seen_at"].is_number());
    let last_seen = credentials::heartbeat(&pool, &connection).await.unwrap();

    let replacement = issue(&pool, NODE).await;
    let revoked = serde_json::to_value(model::get(&pool, NODE).await.unwrap()).unwrap();
    assert_eq!(revoked["status"], "offline");
    assert_eq!(revoked["last_seen_at"], last_seen);
    assert_eq!(
        credentials::heartbeat(&pool, &connection)
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        credentials::authenticate(&pool, FIRST_DEVICE, Some(&replacement.token))
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::UNAUTHORIZED
    );

    let candidate = credentials::authenticate(&pool, SECOND_DEVICE, Some(&replacement.token))
        .await
        .unwrap();
    let (connection, _) = credentials::connect(&pool, candidate, metadata)
        .await
        .unwrap();
    assert_eq!(connection.node_id, NODE);
    let nodes = serde_json::to_value(
        model::list(&pool, 50, 0, &Default::default())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(nodes["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(nodes["has_more"], false);
}

#[tokio::test]
async fn invalid_enrollment_tokens_cannot_claim_a_node() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    seed_node(&pool, NODE).await;
    seed_node(&pool, OTHER_NODE).await;

    let expired = issue(&pool, NODE).await;
    sqlx::query(
        "UPDATE node_enrollment_tokens SET issued_at = unixepoch() - 100,
         expires_at = unixepoch() - 1 WHERE node_id = ?",
    )
    .bind(NODE)
    .execute(&pool)
    .await
    .unwrap();

    let revoked = issue(&pool, OTHER_NODE).await;
    let mut tx = pool.begin().await.unwrap();
    credentials::revoke(&mut tx, OTHER_NODE).await.unwrap();
    tx.commit().await.unwrap();

    for token in [expired.token, revoked.token] {
        let error = credentials::authenticate(&pool, FIRST_DEVICE, Some(&token))
            .await
            .err()
            .unwrap();
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    let enrollment = issue(&pool, NODE).await;
    let candidate = credentials::authenticate(&pool, FIRST_DEVICE, Some(&enrollment.token))
        .await
        .unwrap();
    let mut token_race = pool.begin().await.unwrap();
    credentials::revoke(&mut token_race, NODE).await.unwrap();
    token_race.commit().await.unwrap();
    let hello = decode(&hello().to_string()).unwrap();
    let metadata = validate_hello(&hello, NODE).unwrap();
    assert_eq!(
        credentials::connect(&pool, candidate, metadata)
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::UNAUTHORIZED
    );

    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agent_credentials")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_late_handshake_cannot_replace_the_current_agent_route() {
    use crate::agent_bus::{AgentBus, Control};
    use std::sync::Arc;
    use std::time::Duration;

    let pool = database::connect("sqlite::memory:").await.unwrap();
    seed_node(&pool, NODE).await;
    let issued = issue(&pool, NODE).await;
    let envelope = decode(&hello().to_string()).unwrap();
    let metadata = validate_hello(&envelope, NODE).unwrap();
    let first = credentials::authenticate(&pool, FIRST_DEVICE, Some(&issued.token))
        .await
        .unwrap();
    let (old, _) = credentials::connect(&pool, first, metadata).await.unwrap();
    let next = credentials::authenticate(&pool, FIRST_DEVICE, None)
        .await
        .unwrap();
    let (current, _) = credentials::connect(&pool, next, metadata).await.unwrap();

    let bus = Arc::new(AgentBus::new(15));
    let mut subscription = super::stream::subscribe(&pool, &current, &bus, true)
        .await
        .unwrap();
    assert!(matches!(
        super::stream::subscribe(&pool, &old, &bus, false).await,
        Err(SocketFailure::Policy)
    ));
    let task_id = "0123456789abcdef0123456789abcdef";
    let task = bus
        .enqueue(
            NODE,
            task_id,
            "processes",
            json!({}),
            Duration::from_secs(1),
        )
        .unwrap();
    let Control::Task(dispatch) = subscription.next().await else {
        panic!("expected task dispatch to the current connection");
    };
    assert_eq!(dispatch.request.request_id, task_id);
    drop(task);
}

#[test]
fn agent_message_checks_version_identity_replay_and_shape() {
    let mut raw = hello();
    raw["protocol_version"] = json!(PROTOCOL_VERSION + 1);
    let envelope = decode(&raw.to_string()).unwrap();
    assert_eq!(
        validate_hello(&envelope, NODE),
        Err(SocketFailure::Protocol)
    );

    raw["protocol_version"] = json!(PROTOCOL_VERSION);
    raw["node_id"] = json!(OTHER_NODE);
    let mut envelope = decode(&raw.to_string()).unwrap();
    assert_eq!(validate_hello(&envelope, NODE), Err(SocketFailure::Policy));
    envelope.node_id = Some(NODE.to_owned());
    assert!(validate_hello(&envelope, NODE).is_ok());

    envelope.message = AgentMessage::Heartbeat {};
    envelope.message_id = 5;
    assert!(validate_heartbeat(&envelope, NODE, 4).is_ok());
    assert_eq!(
        validate_heartbeat(&envelope, NODE, 5),
        Err(SocketFailure::Policy)
    );
    assert_eq!(
        validate_heartbeat(&envelope, OTHER_NODE, 4),
        Err(SocketFailure::Policy)
    );

    raw["unexpected"] = json!(true);
    assert_eq!(decode(&raw.to_string()), Err(SocketFailure::Protocol));

    raw.as_object_mut().unwrap().remove("unexpected");
    raw["payload"]["unexpected"] = json!(true);
    assert_eq!(decode(&raw.to_string()), Err(SocketFailure::Protocol));

    raw["type"] = json!("heartbeat");
    raw["payload"] = json!({ "unexpected": true });
    assert_eq!(decode(&raw.to_string()), Err(SocketFailure::Protocol));
}
