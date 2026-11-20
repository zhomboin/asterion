use std::sync::Arc;

use asterion_protocol::is_node_id;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{SqliteConnection, SqlitePool};

use super::ApiError;
use crate::auth::AdminSession;
use crate::ssh_bus::SshBus;

const WRITE_MIN_SECONDS: i64 = 5;
const MAX_SAMPLE_BYTES: usize = 48 * 1024;
const MAX_HISTORY_BYTES: i64 = 4 * 1024 * 1024;

pub async fn store_metrics(
    connection: &mut SqliteConnection,
    node_id: &str,
    sample: &Value,
) -> Result<(), sqlx::Error> {
    let Some(encoded) = metrics_sample(sample) else {
        return Ok(());
    };
    let observed_at = sqlx::query_scalar::<_, i64>(
        "INSERT OR IGNORE INTO metric_samples (node_id, observed_at, sample)
         SELECT ?, unixepoch(), ? WHERE NOT EXISTS (
             SELECT 1 FROM metric_samples
             WHERE node_id = ? AND observed_at > unixepoch() - ? AND observed_at <= unixepoch()
         ) RETURNING observed_at",
    )
    .bind(node_id)
    .bind(encoded)
    .bind(node_id)
    .bind(WRITE_MIN_SECONDS)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(observed_at) = observed_at {
        crate::trends::observe(connection, node_id, observed_at, sample).await?;
    }
    Ok(())
}

pub async fn purge_metrics(pool: &SqlitePool, retention_seconds: i64) -> Result<u64, sqlx::Error> {
    if retention_seconds <= 0 {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM metric_samples WHERE observed_at < unixepoch() - ?")
        .bind(retention_seconds)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub(crate) fn valid_sample(value: &Value) -> bool {
    metrics_sample(value).is_some()
}

fn metrics_sample(sample: &Value) -> Option<String> {
    let map = sample.as_object()?;
    if map.len() != 8
        || !map
            .get("captured_at")
            .and_then(Value::as_i64)
            .is_some_and(|time| time >= 0)
        || !map
            .get("logical_cores")
            .and_then(Value::as_u64)
            .is_some_and(|cores| cores > 0 && cores <= 262_144)
        || !map.get("cpu_percent").is_some_and(percent)
        || !map
            .get("load")
            .and_then(Value::as_array)
            .is_some_and(|load| load.len() == 3 && load.iter().all(nonnegative_number))
        || !map.get("memory").is_some_and(memory)
        || !map.get("swap").is_some_and(memory)
        || !map
            .get("filesystems")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() <= 256 && items.iter().all(filesystem))
        || !map
            .get("interfaces")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() <= 256 && items.iter().all(interface))
    {
        return None;
    }
    let encoded = sample.to_string();
    (encoded.len() <= MAX_SAMPLE_BYTES).then_some(encoded)
}

fn percent(value: &Value) -> bool {
    value.is_null()
        || value
            .as_f64()
            .is_some_and(|number| (0.0..=100.0).contains(&number))
}

fn nonnegative_number(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number >= 0.0 && number.is_finite())
}

fn memory(value: &Value) -> bool {
    let Some(total) = value.get("total_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(available) = value.get("available_bytes").and_then(Value::as_u64) else {
        return false;
    };
    value.as_object().is_some_and(|map| map.len() == 3)
        && total
            .checked_sub(available)
            .is_some_and(|used| value.get("used_bytes").and_then(Value::as_u64) == Some(used))
}

fn filesystem(value: &Value) -> bool {
    let total = value.get("total_bytes").and_then(Value::as_u64);
    let available = value.get("available_bytes").and_then(Value::as_u64);
    value.as_object().is_some_and(|map| map.len() == 4)
        && bounded_string(value.get("mountpoint"), 4096)
        && bounded_string(value.get("fs_type"), 100)
        && matches!((total, available), (Some(total), Some(available)) if available <= total)
}

fn interface(value: &Value) -> bool {
    value.as_object().is_some_and(|map| map.len() == 5)
        && bounded_string(value.get("name"), 256)
        && ["rx_bytes", "tx_bytes"]
            .iter()
            .all(|name| value.get(name).and_then(Value::as_u64).is_some())
        && ["rx_bytes_per_second", "tx_bytes_per_second"]
            .iter()
            .all(|name| {
                value
                    .get(name)
                    .is_some_and(|number| number.is_null() || nonnegative_number(number))
            })
}

fn bounded_string(value: Option<&Value>, limit: usize) -> bool {
    value.and_then(Value::as_str).is_some_and(|value| {
        !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryQuery {
    range: Option<String>,
    limit: Option<u32>,
}

#[derive(Serialize)]
pub(crate) struct History {
    range: &'static str,
    samples: Vec<ObservedSample>,
    bucket_seconds: i64,
    interval_seconds: u64,
    truncated: bool,
}

#[derive(Serialize)]
struct ObservedSample {
    observed_at: i64,
    sample: Value,
}

pub(crate) async fn history(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<SshBus>>,
    Path(node_id): Path<String>,
    query: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<History>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let (range, seconds) = match query.range.as_deref().unwrap_or("1h") {
        "1h" => ("1h", 60_i64 * 60),
        "24h" => ("24h", 24 * 60 * 60),
        _ => return Err(ApiError::INVALID),
    };
    let limit = query.limit.unwrap_or(360);
    if !(1..=720).contains(&limit) {
        return Err(ApiError::INVALID);
    }
    ensure_node(&pool, &node_id).await?;

    let bucket = (seconds / i64::from(limit)).max(WRITE_MIN_SECONDS);
    let rows = sqlx::query_as::<_, (i64, Option<String>)>(
        "WITH buckets AS (
             SELECT MAX(observed_at) AS observed_at FROM metric_samples
             WHERE node_id = ? AND observed_at >= unixepoch() - ?
             GROUP BY observed_at / ?
             ORDER BY observed_at DESC LIMIT ?
         ), history AS (
             SELECT sample.observed_at, sample.sample,
                SUM(length(CAST(sample.sample AS BLOB)) + 64)
                OVER (ORDER BY sample.observed_at DESC) AS response_bytes
             FROM buckets
             JOIN metric_samples sample ON sample.node_id = ? AND sample.observed_at = buckets.observed_at
         )
         SELECT observed_at, CASE WHEN response_bytes <= ? THEN sample END
         FROM history ORDER BY observed_at DESC",
    )
    .bind(&node_id)
    .bind(seconds)
    .bind(bucket)
    .bind(limit)
    .bind(&node_id)
    .bind(MAX_HISTORY_BYTES)
    .fetch_all(&pool)
    .await?;

    let mut truncated = false;
    let mut samples = Vec::with_capacity(rows.len());
    for (observed_at, encoded) in rows {
        let Some(encoded) = encoded else {
            truncated = true;
            break;
        };
        let sample = serde_json::from_str::<Value>(&encoded).map_err(sqlx::Error::decode)?;
        samples.push(ObservedSample {
            observed_at,
            sample,
        });
    }
    samples.reverse();
    Ok(Json(History {
        range,
        samples,
        bucket_seconds: bucket,
        interval_seconds: bus.sample_interval_secs(),
        truncated,
    }))
}

pub(crate) async fn ensure_node(pool: &SqlitePool, id: &str) -> Result<(), ApiError> {
    if !is_node_id(id) {
        return Err(ApiError::NODE_NOT_FOUND);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(ApiError::NODE_NOT_FOUND);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn samples_are_validated_rate_bounded_and_purged() {
        let pool = crate::database::connect("sqlite::memory:").await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let id = "0".repeat(32);
        sqlx::query("INSERT INTO nodes (id, name, kind) VALUES (?, '测试', 'linux')")
            .bind(&id)
            .execute(&mut *conn)
            .await
            .unwrap();
        let sample = json!({
            "captured_at": 0, "logical_cores": 4, "cpu_percent": null, "load": [0, 0, 0],
            "memory": {"total_bytes":100,"available_bytes":98,"used_bytes":2},
            "swap": {"total_bytes":0,"available_bytes":0,"used_bytes":0},
            "filesystems": [], "interfaces": []
        });
        store_metrics(&mut conn, &id, &json!({"captured_at":0}))
            .await
            .unwrap();
        for _ in 0..2 {
            store_metrics(&mut conn, &id, &sample).await.unwrap();
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metric_samples")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(count, 1);
        sqlx::query("UPDATE metric_samples SET observed_at = unixepoch() - 86400")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
        assert_eq!(purge_metrics(&pool, 3_600).await.unwrap(), 1);
    }
}
