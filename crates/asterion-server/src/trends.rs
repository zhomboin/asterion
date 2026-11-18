use asterion_protocol::is_node_id;
use axum::Json;
use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::auth::AdminSession;
use crate::monitor::ApiError;

const BUCKET_SECONDS: i64 = 300;

pub fn routes() -> Router<SqlitePool> {
    Router::new().route("/api/v1/nodes/{id}/trends", get(history))
}

#[derive(Debug)]
pub(crate) struct Statistics {
    pub cpu: Option<f64>,
    pub memory: Option<f64>,
    pub disk: Option<f64>,
    pub rx: Option<f64>,
    pub tx: Option<f64>,
}

pub(crate) fn statistics(sample: &Value) -> Statistics {
    let memory = sample.get("memory");
    let total = memory.and_then(|value| value["total_bytes"].as_f64());
    let used = memory.and_then(|value| value["used_bytes"].as_f64());
    let filesystems = sample["filesystems"].as_array().into_iter().flatten();
    let disk = filesystems
        .filter_map(|fs| {
            let total = fs["total_bytes"].as_f64()?;
            let available = fs["available_bytes"].as_f64()?;
            percentage(total - available, total)
        })
        .reduce(f64::max);

    let sum = |key: &str| {
        sample["interfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|interface| interface[key].as_f64())
            .reduce(|total, value| total + value)
            .filter(|number| number.is_finite())
    };
    Statistics {
        cpu: sample["cpu_percent"].as_f64(),
        memory: used
            .zip(total)
            .and_then(|(used, total)| percentage(used, total)),
        disk,
        rx: sum("rx_bytes_per_second"),
        tx: sum("tx_bytes_per_second"),
    }
}

fn percentage(used: f64, total: f64) -> Option<f64> {
    (total > 0.0 && used >= 0.0 && used <= total).then(|| used / total * 100.0)
}

pub(crate) async fn observe(
    connection: &mut SqliteConnection,
    node: &str,
    observed_at: i64,
    sample: &Value,
) -> Result<(), sqlx::Error> {
    let current = statistics(sample);
    sqlx::query(
        "INSERT INTO metric_rollups (node_id, bucket_at, sample_count, cpu_sum, cpu_count,
            memory_sum, memory_count, disk_sum, disk_count, rx_sum, rx_count, tx_sum, tx_count)
         VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (node_id, bucket_at) DO UPDATE SET
            sample_count = sample_count + 1, cpu_sum = cpu_sum + excluded.cpu_sum,
            cpu_count = cpu_count + excluded.cpu_count, memory_sum = memory_sum + excluded.memory_sum,
            memory_count = memory_count + excluded.memory_count, disk_sum = disk_sum + excluded.disk_sum,
            disk_count = disk_count + excluded.disk_count, rx_sum = rx_sum + excluded.rx_sum,
            rx_count = rx_count + excluded.rx_count, tx_sum = tx_sum + excluded.tx_sum,
            tx_count = tx_count + excluded.tx_count",
    )
    .bind(node)
    .bind(observed_at.div_euclid(BUCKET_SECONDS) * BUCKET_SECONDS)
    .bind(current.cpu.unwrap_or_default())
    .bind(current.cpu.is_some())
    .bind(current.memory.unwrap_or_default())
    .bind(current.memory.is_some())
    .bind(current.disk.unwrap_or_default())
    .bind(current.disk.is_some())
    .bind(current.rx.unwrap_or_default())
    .bind(current.rx.is_some())
    .bind(current.tx.unwrap_or_default())
    .bind(current.tx.is_some())
    .execute(connection)
    .await?;
    Ok(())
}

pub async fn purge(pool: &SqlitePool, retention_seconds: i64) -> Result<u64, sqlx::Error> {
    if retention_seconds <= 0 {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM metric_rollups WHERE bucket_at < unixepoch() - ?")
        .bind(retention_seconds)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    range: Option<String>,
}

#[derive(Serialize)]
struct Trend {
    range: &'static str,
    bucket_seconds: i64,
    samples: Vec<TrendSample>,
}

#[derive(FromRow, Serialize)]
struct TrendSample {
    observed_at: i64,
    cpu_percent: Option<f64>,
    memory_percent: Option<f64>,
    disk_percent: Option<f64>,
    rx_bytes_per_second: Option<f64>,
    tx_bytes_per_second: Option<f64>,
    sample_count: i64,
}

async fn history(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(node): Path<String>,
    options: Result<Query<Filter>, QueryRejection>,
) -> Result<Json<Trend>, ApiError> {
    let Query(options) = options.map_err(|_| ApiError::INVALID)?;
    let (range, seconds) = match options.range.as_deref().unwrap_or("7d") {
        "7d" => ("7d", 7 * 86_400_i64),
        "30d" => ("30d", 30 * 86_400),
        _ => return Err(ApiError::INVALID),
    };
    if !is_node_id(&node)
        || !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
            .bind(&node)
            .fetch_one(&pool)
            .await?
    {
        return Err(ApiError::NODE_NOT_FOUND);
    }
    let samples = sqlx::query_as::<_, TrendSample>(
        "SELECT bucket_at AS observed_at, cpu_sum / NULLIF(cpu_count, 0) AS cpu_percent,
            memory_sum / NULLIF(memory_count, 0) AS memory_percent,
            disk_sum / NULLIF(disk_count, 0) AS disk_percent,
            rx_sum / NULLIF(rx_count, 0) AS rx_bytes_per_second,
            tx_sum / NULLIF(tx_count, 0) AS tx_bytes_per_second, sample_count
         FROM metric_rollups WHERE node_id = ? AND bucket_at >= ((unixepoch() - ?) / 300) * 300
         AND bucket_at <= unixepoch() ORDER BY bucket_at LIMIT 8641",
    )
    .bind(node)
    .bind(seconds)
    .fetch_all(&pool)
    .await?;
    Ok(Json(Trend {
        range,
        bucket_seconds: BUCKET_SECONDS,
        samples,
    }))
}
