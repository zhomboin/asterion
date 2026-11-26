use asterion_protocol::MAX_SAMPLE_INTERVAL_SECS;
use serde_json::Value;
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::trends;

const EVALUATION_BATCH: i64 = 24;
const MAX_METRIC_DELTA: i64 = 128;

#[derive(FromRow)]
struct Rule {
    id: String,
    node_id: String,
    node_name: String,
    metric: String,
    threshold: Option<f64>,
    for_seconds: i64,
    breach_since: Option<i64>,
    last_sample_at: Option<i64>,
    ssh_online_until: Option<i64>,
    collector_interval: Option<u64>,
    now: i64,
}

pub async fn evaluate(pool: &SqlitePool, sample_interval_secs: u64) -> Result<(), sqlx::Error> {
    let mut after = String::new();
    loop {
        let mut tx = pool.begin().await?;
        let rules = sqlx::query_as::<_, Rule>(
            "SELECT r.*, n.name AS node_name, n.ssh_online_until, c.interval_secs AS collector_interval, unixepoch() AS now FROM alert_rules r
             JOIN nodes n ON n.id = r.node_id LEFT JOIN collector_sync c ON c.node_id=n.id AND c.active=1 WHERE r.enabled = 1 AND r.id > ? ORDER BY r.id LIMIT ?",
        )
        .bind(&after)
        .bind(EVALUATION_BATCH)
        .fetch_all(&mut *tx)
        .await?;
        for rule in &rules {
            if rule.metric == "offline" {
                offline(&mut tx, rule).await?;
            } else {
                let interval = rule.collector_interval.unwrap_or(sample_interval_secs);
                let max_gap = (interval.clamp(5, MAX_SAMPLE_INTERVAL_SECS) as i64 * 2).max(45);
                metric(&mut tx, rule, max_gap).await?;
            }
        }
        tx.commit().await?;
        if rules.len() < EVALUATION_BATCH as usize {
            return Ok(());
        }
        rules
            .last()
            .expect("full batch has a last rule")
            .id
            .clone_into(&mut after);
    }
}

async fn offline(connection: &mut SqliteConnection, rule: &Rule) -> Result<(), sqlx::Error> {
    let Some(until) = rule.ssh_online_until else {
        return Ok(());
    };
    let offline_for = rule.now - until;
    if offline_for < 0 {
        close_event(connection, rule, rule.now).await?;
    } else if offline_for >= rule.for_seconds {
        open_event(connection, rule, None, rule.now).await?;
    }
    Ok(())
}

async fn metric(
    connection: &mut SqliteConnection,
    rule: &Rule,
    max_gap: i64,
) -> Result<(), sqlx::Error> {
    let Some(threshold) = rule.threshold else {
        return Ok(());
    };
    let mut rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT observed_at, sample FROM metric_samples
         WHERE node_id = ? AND observed_at <= ? AND observed_at >= ?
         AND observed_at > COALESCE(?, (
             SELECT MAX(observed_at) - 1 FROM metric_samples WHERE node_id = ? AND observed_at <= ?
         )) ORDER BY observed_at DESC LIMIT ?",
    )
    .bind(&rule.node_id)
    .bind(rule.now)
    .bind(rule.now - max_gap)
    .bind(rule.last_sample_at)
    .bind(&rule.node_id)
    .bind(rule.now)
    .bind(MAX_METRIC_DELTA)
    .fetch_all(&mut *connection)
    .await?;

    let mut from = rule.breach_since;
    let mut previous = rule.last_sample_at;
    if rows.len() == MAX_METRIC_DELTA as usize
        || previous.is_some_and(|sample| sample < rule.now - max_gap || sample > rule.now)
    {
        from = None;
        previous = None;
    }

    rows.reverse();
    for (observed, encoded) in rows {
        let metrics = serde_json::from_str::<Value>(&encoded).map_err(sqlx::Error::decode)?;
        let stats = trends::statistics(&metrics);
        let value = match rule.metric.as_str() {
            "cpu" => stats.cpu,
            "memory" => stats.memory,
            "disk" => stats.disk,
            _ => None,
        };
        if previous.is_none_or(|sample| observed - sample > max_gap) {
            from = None;
        }
        previous = Some(observed);

        match value {
            Some(value) if value >= threshold => {
                let since = *from.get_or_insert(observed);
                if observed - since >= rule.for_seconds {
                    open_event(connection, rule, Some(value), observed).await?;
                }
            }
            Some(_) => {
                from = None;
                close_event(connection, rule, observed).await?;
            }
            None => {
                // 数据缺失中断持续计时，但不能把已出现的告警视为恢复。
                from = None;
            }
        }
    }

    if (from, previous) != (rule.breach_since, rule.last_sample_at) {
        sqlx::query("UPDATE alert_rules SET breach_since = ?, last_sample_at = ? WHERE id = ?")
            .bind(from)
            .bind(previous)
            .bind(&rule.id)
            .execute(connection)
            .await?;
    }
    Ok(())
}

async fn open_event(
    connection: &mut SqliteConnection,
    rule: &Rule,
    value: Option<f64>,
    observed_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO alert_events (id, rule_id, node_id, node_name, metric, threshold, value, opened_at, updated_at)
         VALUES (lower(hex(randomblob(16))), ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (rule_id) WHERE resolved_at IS NULL DO UPDATE SET
             value = excluded.value, updated_at = MAX(alert_events.updated_at, excluded.updated_at)",
    )
    .bind(&rule.id)
    .bind(&rule.node_id)
    .bind(&rule.node_name)
    .bind(&rule.metric)
    .bind(rule.threshold)
    .bind(value)
    .bind(observed_at)
    .bind(observed_at)
    .execute(connection)
    .await?;
    Ok(())
}

async fn close_event(
    connection: &mut SqliteConnection,
    rule: &Rule,
    observed_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE alert_events SET updated_at = MAX(updated_at, ?), resolved_at = MAX(updated_at, ?)
         WHERE rule_id = ? AND resolved_at IS NULL",
    )
    .bind(observed_at)
    .bind(observed_at)
    .bind(&rule.id)
    .execute(connection)
    .await?;
    Ok(())
}
