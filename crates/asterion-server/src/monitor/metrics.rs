use std::sync::Arc;

use asterion_protocol::{collector::metrics_sample, is_node_id};
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
    collector: Option<CollectorStatus>,
}

#[derive(Serialize, sqlx::FromRow)]
struct CollectorStatus {
    interval_secs: u64,
    pending: i64,
    missed: i64,
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
    let collector: Option<CollectorStatus> = sqlx::query_as(
        "SELECT interval_secs, MAX(0,newest_seq-seq) AS pending, missed FROM collector_sync WHERE node_id=? AND active=1"
    ).bind(&node_id).fetch_optional(&pool).await?;
    Ok(Json(History {
        range,
        samples,
        bucket_seconds: bucket,
        interval_seconds: collector
            .as_ref()
            .map_or(bus.sample_interval_secs(), |status| status.interval_secs),
        collector,
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
