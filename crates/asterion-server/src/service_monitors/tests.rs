use std::sync::Arc;
use std::time::Duration;

use asterion_protocol::TaskReply;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::polling::{interpret, poll_due};
use super::storage::{DueMonitor, Observation, disable, due, enable, history, list, purge, record};
use crate::database;
use crate::ssh_bus::{BusError, Control, SshBus};

const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_NODE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const UNIT: &str = "demo.service";

async fn pool() -> SqlitePool {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    for id in [NODE, OTHER_NODE] {
        sqlx::query("INSERT INTO nodes (id, name, kind) VALUES (?, '测试节点', 'linux')")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool
}

fn reply(unit: &str, active: &str, sub: &str) -> Value {
    json!({ "name": unit, "source": "systemd", "verified": true, "active": active, "sub": sub })
}

#[test]
fn only_verified_exact_units_confirm_service_status() {
    for (active, sub, expected) in [
        ("active", "exited", "healthy"),
        ("reloading", "reload", "healthy"),
        ("refreshing", "reload", "healthy"),
        ("failed", "failed", "down"),
        ("inactive", "dead", "down"),
        ("activating", "start", "unknown"),
        ("deactivating", "stop", "unknown"),
    ] {
        let value = interpret(UNIT, Ok(reply(UNIT, active, sub)));
        assert_eq!(value.status, expected);
        assert_eq!(value.active.as_deref(), Some(active));
        assert_eq!(value.sub.as_deref(), Some(sub));
    }

    for mut value in [
        reply("other.service", "failed", "failed"),
        reply(UNIT, "inactive", "newline\n"),
        reply(UNIT, "active", ""),
    ] {
        assert_eq!(
            interpret(UNIT, Ok(value.clone())),
            Observation::unknown("invalid_response")
        );
        value["verified"] = json!(false);
        assert_eq!(
            interpret(UNIT, Ok(value)),
            Observation::unknown("invalid_response")
        );
    }
    assert_eq!(
        interpret(UNIT, Err(BusError::Offline)),
        Observation::unknown("offline"),
    );
    assert!(super::validate_target(NODE, "demo@blue\\x2dgreen.service").is_ok());
    for unit in [
        "demo@.service",
        "demo*.service",
        "demo.socket",
        "-demo.service",
    ] {
        assert!(super::validate_target(NODE, unit).is_err());
    }
    assert!(super::validate_target("unknown-node", UNIT).is_err());
}

#[tokio::test]
async fn scheduler_records_down_and_recovery_without_resolving_on_agent_loss() {
    let pool = pool().await;
    let bus = Arc::new(SshBus::new(15));
    enable(&pool, NODE, UNIT).await.unwrap();
    assert_eq!(list(&pool, NODE).await.unwrap()[0].status, "pending");
    let mut agent = bus.attach(NODE, true);
    for active in ["failed", "failed"] {
        mark_due(&pool).await;
        tokio::join!(async { poll_due(&pool, &bus).await.unwrap() }, async {
            let Control::Task(task) = tokio::time::timeout(Duration::from_secs(3), agent.next())
                .await
                .unwrap()
            else {
                panic!("expected the systemd read request");
            };
            assert_eq!(&task.request.kind, "systemd_service");
            assert_eq!(task.request.args, json!({"unit": UNIT}));
            task.response
                .send(TaskReply::Success {
                    value: reply(UNIT, active, "failed"),
                })
                .unwrap();
        });
    }
    assert_eq!(events(&pool).await, vec![(None,)]);

    drop(agent);
    mark_due(&pool).await;
    poll_due(&pool, &bus).await.unwrap();
    assert_eq!(
        list(&pool, NODE).await.unwrap()[0].reason.as_deref(),
        Some("offline")
    );
    assert_eq!(events(&pool).await, vec![(None,)]);
    assert_eq!(history(&pool, NODE, UNIT).await.unwrap().len(), 2);

    let mut agent = bus.attach(NODE, true);
    mark_due(&pool).await;
    tokio::join!(async { poll_due(&pool, &bus).await.unwrap() }, async {
        let Control::Task(task) = tokio::time::timeout(Duration::from_secs(3), agent.next())
            .await
            .unwrap()
        else {
            panic!("expected the systemd read request");
        };
        task.response
            .send(TaskReply::Success {
                value: reply(UNIT, "active", "running"),
            })
            .unwrap();
    });

    assert_eq!(list(&pool, NODE).await.unwrap()[0].status, "healthy");
    assert_eq!(events(&pool).await, vec![(Some("recovered".into()),)]);
    assert_eq!(
        history(&pool, NODE, UNIT)
            .await
            .unwrap()
            .iter()
            .map(|item| item.status.as_str())
            .collect::<Vec<_>>(),
        ["healthy", "unknown", "down"],
    );
}

#[tokio::test]
async fn configuration_races_keep_old_results_and_duplicate_toggles_out() {
    let pool = pool().await;
    let monitor = enable_due(&pool).await;
    let failed = interpret(UNIT, Ok(reply(UNIT, "failed", "failed")));
    record(&pool, &monitor, &failed).await.unwrap();

    let current = enable(&pool, NODE, UNIT).await.unwrap();
    assert_eq!(current.status, "down"); // 幂等启用不重置状态。
    let paused = disable(&pool, NODE, UNIT).await.unwrap();
    assert!(!paused.enabled);
    assert!(!disable(&pool, NODE, UNIT).await.unwrap().enabled);
    assert_eq!(events(&pool).await, vec![(Some("disabled".into()),)]);
    assert_eq!(history(&pool, NODE, UNIT).await.unwrap().len(), 1);

    assert!(enable(&pool, NODE, UNIT).await.unwrap().enabled);
    record(&pool, &monitor, &failed).await.unwrap(); // 上一代在途结果。
    assert_eq!(list(&pool, NODE).await.unwrap()[0].status, "pending");
    assert_eq!(events(&pool).await.len(), 1);

    let latest = due(&pool, 1, 1).await.unwrap().remove(0);
    assert!(latest.generation > monitor.generation);
    record(&pool, &latest, &failed).await.unwrap();
    assert_eq!(history(&pool, NODE, UNIT).await.unwrap().len(), 2);
    assert_eq!(events(&pool).await.len(), 2);
    assert!(history(&pool, OTHER_NODE, UNIT).await.is_err());
}

#[tokio::test]
async fn monitor_limit_and_history_and_alert_retention_are_scoped() {
    let pool = pool().await;
    let watch = enable_due(&pool).await;
    // 已过期的事件中，只有关闭过的事件可以清除。
    let failed = interpret(UNIT, Ok(reply(UNIT, "failed", "failed")));
    record(&pool, &watch, &failed).await.unwrap();
    let healthy = interpret(UNIT, Ok(reply(UNIT, "active", "running")));
    record(&pool, &watch, &healthy).await.unwrap();
    record(&pool, &watch, &failed).await.unwrap();
    sqlx::query(
        "UPDATE service_alert_events SET opened_at = 1, updated_at = 1,
          resolved_at = CASE WHEN resolved_at IS NULL THEN NULL ELSE 1 END",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE service_monitor_history SET observed_at = 1")
        .execute(&pool)
        .await
        .unwrap();

    for index in 0..15 {
        enable(&pool, NODE, &format!("test-{index}.service"))
            .await
            .unwrap();
    }
    assert!(enable(&pool, NODE, "too-many.service").await.is_err());
    assert!(enable(&pool, OTHER_NODE, UNIT).await.is_ok());

    purge(&pool, 3600).await.unwrap();
    assert_eq!(events(&pool).await, vec![(None,)]);
    assert!(history(&pool, NODE, UNIT).await.unwrap().is_empty());
    assert!(list(&pool, NODE).await.unwrap()[0].enabled);

    sqlx::query("DELETE FROM nodes WHERE id = ?")
        .bind(NODE)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(events(&pool).await, vec![(Some("disabled".into()),)]);
    assert_eq!(list(&pool, OTHER_NODE).await.unwrap().len(), 1);
}

async fn enable_due(pool: &SqlitePool) -> DueMonitor {
    enable(pool, NODE, UNIT).await.unwrap();
    due(pool, 1, 1).await.unwrap().remove(0)
}

async fn mark_due(pool: &SqlitePool) {
    sqlx::query("UPDATE service_monitors SET next_check_at = 0")
        .execute(pool)
        .await
        .unwrap();
}

async fn events(pool: &SqlitePool) -> Vec<(Option<String>,)> {
    sqlx::query_as("SELECT resolution FROM service_alert_events ORDER BY rowid")
        .fetch_all(pool)
        .await
        .unwrap()
}
