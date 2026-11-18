use axum::http::StatusCode;
use serde::Serialize;
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use super::NOT_FOUND;
use crate::monitor::ApiError;

const MAX_ENABLED_PER_NODE: i64 = 16;
const HISTORY_LIMIT: i64 = 50;
const CHECK_INTERVAL_SECS: i64 = 15;
const MONITOR_FIELDS: &str = "unit, enabled, status, active, sub, checked_at, changed_at, reason";
const MONITOR_LIMIT: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "service_monitor_limit",
    "该节点最多可同时监控 16 个服务",
);

#[derive(FromRow, Serialize)]
pub(super) struct Monitor {
    pub unit: String,
    pub enabled: bool,
    pub status: String,
    pub active: Option<String>,
    pub sub: Option<String>,
    pub checked_at: Option<i64>,
    pub changed_at: Option<i64>,
    pub reason: Option<String>,
}

#[derive(FromRow, Serialize)]
pub(super) struct HistoryEntry {
    pub id: i64,
    pub status: String,
    pub active: Option<String>,
    pub sub: Option<String>,
    pub observed_at: i64,
    pub reason: Option<String>,
}

#[derive(FromRow)]
pub(super) struct DueMonitor {
    pub id: String,
    pub node_id: String,
    pub unit: String,
    pub generation: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub status: &'static str,
    pub active: Option<String>,
    pub sub: Option<String>,
    pub reason: Option<&'static str>,
}

impl Observation {
    pub fn unknown(reason: &'static str) -> Self {
        Self {
            status: "unknown",
            active: None,
            sub: None,
            reason: Some(reason),
        }
    }
}

pub(super) async fn list(pool: &SqlitePool, node_id: &str) -> Result<Vec<Monitor>, sqlx::Error> {
    let query = format!(
        "SELECT {MONITOR_FIELDS} FROM service_monitors WHERE node_id = ? ORDER BY enabled DESC, unit"
    );
    sqlx::query_as(&query).bind(node_id).fetch_all(pool).await
}

pub(super) async fn enable(
    pool: &SqlitePool,
    node_id: &str,
    unit: &str,
) -> Result<Monitor, ApiError> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_node(&mut transaction, node_id).await?;
    if let Some(current) = get(&mut transaction, node_id, unit).await?
        && current.enabled
    {
        transaction.commit().await?;
        return Ok(current);
    }

    let enabled = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM service_monitors WHERE node_id = ? AND enabled = 1",
    )
    .bind(node_id)
    .fetch_one(&mut *transaction)
    .await?;
    if enabled >= MAX_ENABLED_PER_NODE {
        return Err(MONITOR_LIMIT);
    }

    let query = format!(
        "INSERT INTO service_monitors (node_id, unit) VALUES (?, ?)
         ON CONFLICT (node_id, unit) DO UPDATE
         SET enabled = 1, generation = generation + 1, disabled_at = NULL,
             status = 'pending', active = NULL, sub = NULL, reason = NULL,
             checked_at = NULL, changed_at = NULL, next_check_at = unixepoch()
         RETURNING {MONITOR_FIELDS}"
    );
    let monitor = sqlx::query_as(&query)
        .bind(node_id)
        .bind(unit)
        .fetch_one(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(monitor)
}

pub(super) async fn disable(
    pool: &SqlitePool,
    node_id: &str,
    unit: &str,
) -> Result<Monitor, ApiError> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    require_node(&mut transaction, node_id).await?;
    let changed = sqlx::query_scalar::<_, String>(
        "UPDATE service_monitors
         SET enabled = 0, generation = generation + 1, disabled_at = unixepoch()
         WHERE node_id = ? AND unit = ? AND enabled = 1 RETURNING id",
    )
    .bind(node_id)
    .bind(unit)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(id) = changed {
        close_alert(&mut transaction, &id, "disabled").await?;
    }
    let monitor = get(&mut transaction, node_id, unit)
        .await?
        .ok_or(NOT_FOUND)?;
    transaction.commit().await?;
    Ok(monitor)
}

async fn get(
    connection: &mut SqliteConnection,
    node_id: &str,
    unit: &str,
) -> Result<Option<Monitor>, sqlx::Error> {
    let query =
        format!("SELECT {MONITOR_FIELDS} FROM service_monitors WHERE node_id = ? AND unit = ?");
    sqlx::query_as(&query)
        .bind(node_id)
        .bind(unit)
        .fetch_optional(connection)
        .await
}

async fn require_node(connection: &mut SqliteConnection, node_id: &str) -> Result<(), ApiError> {
    let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(node_id)
        .fetch_one(connection)
        .await?;
    if !exists {
        return Err(ApiError::NODE_NOT_FOUND);
    }
    Ok(())
}

pub(super) async fn history(
    pool: &SqlitePool,
    node_id: &str,
    unit: &str,
) -> Result<Vec<HistoryEntry>, ApiError> {
    let mut transaction = pool.begin().await?;
    let id = sqlx::query_scalar::<_, String>(
        "SELECT id FROM service_monitors WHERE node_id = ? AND unit = ?",
    )
    .bind(node_id)
    .bind(unit)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(NOT_FOUND)?;
    let history = sqlx::query_as(
        "SELECT id, status, active, sub, reason, observed_at
         FROM service_monitor_history WHERE monitor_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(id)
    .bind(HISTORY_LIMIT)
    .fetch_all(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(history)
}

pub(super) async fn due(
    pool: &SqlitePool,
    max_nodes: i64,
    max_per_node: i64,
) -> Result<Vec<DueMonitor>, sqlx::Error> {
    sqlx::query_as(
        "WITH pending AS (
             SELECT id, node_id, unit, generation, MIN(next_check_at, unixepoch()) AS due_at
             FROM service_monitors
             WHERE enabled = 1 AND (next_check_at <= unixepoch() OR next_check_at > unixepoch() + ?1)
         ), next_nodes AS (
             SELECT node_id FROM pending GROUP BY node_id ORDER BY MIN(due_at), node_id LIMIT ?2
         ), due_monitors AS (
             SELECT id, node_id, unit, generation,
                    row_number() OVER (PARTITION BY node_id ORDER BY due_at, id) AS position
             FROM pending WHERE node_id IN (SELECT node_id FROM next_nodes)
         )
         SELECT id, node_id, unit, generation FROM due_monitors
         WHERE position <= ?3 ORDER BY node_id, position",
    )
    .bind(CHECK_INTERVAL_SECS)
    .bind(max_nodes)
    .bind(max_per_node)
    .fetch_all(pool)
    .await
}

pub(super) async fn record(
    pool: &SqlitePool,
    monitor: &DueMonitor,
    observation: &Observation,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    // 更新和写入历史、告警处于同一事务。切换监控配置后，旧请求不能影响新代次。
    let current = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        "SELECT status, active, sub, reason FROM service_monitors
         WHERE id = ? AND enabled = 1 AND generation = ?",
    )
    .bind(&monitor.id)
    .bind(monitor.generation)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((status, active, sub, reason)) = current else {
        transaction.commit().await?;
        return Ok(());
    };
    let changed = status != observation.status
        || active != observation.active
        || sub != observation.sub
        || reason.as_deref() != observation.reason;
    let checked_at = sqlx::query_scalar::<_, i64>(
        "UPDATE service_monitors
         SET checked_at = unixepoch(), next_check_at = unixepoch() + ?,
             changed_at = CASE WHEN ? THEN unixepoch() ELSE changed_at END,
             status = ?, active = ?, sub = ?, reason = ?
         WHERE id = ? RETURNING checked_at",
    )
    .bind(CHECK_INTERVAL_SECS)
    .bind(changed)
    .bind(observation.status)
    .bind(&observation.active)
    .bind(&observation.sub)
    .bind(observation.reason)
    .bind(&monitor.id)
    .fetch_one(&mut *transaction)
    .await?;

    if changed {
        sqlx::query(
            "INSERT INTO service_monitor_history (monitor_id, status, active, sub, reason, observed_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&monitor.id)
        .bind(observation.status)
        .bind(&observation.active)
        .bind(&observation.sub)
        .bind(observation.reason)
        .bind(checked_at)
        .execute(&mut *transaction)
        .await?;
    }

    match observation.status {
        "down" => {
            sqlx::query(
                "INSERT INTO service_alert_events (monitor_id, node_id, node_name, unit, opened_at, updated_at)
                 SELECT m.id, n.id, n.name, m.unit, ?1, ?1
                 FROM service_monitors m JOIN nodes n ON n.id = m.node_id WHERE m.id = ?2
                 ON CONFLICT(monitor_id) WHERE resolved_at IS NULL
                 DO UPDATE SET updated_at = excluded.updated_at",
            )
            .bind(checked_at)
            .bind(&monitor.id)
            .execute(&mut *transaction)
            .await?;
        }
        "healthy" => close_alert(&mut transaction, &monitor.id, "recovered").await?,
        _ => {}
    }
    transaction.commit().await?;
    Ok(())
}

async fn close_alert(
    connection: &mut SqliteConnection,
    id: &str,
    resolution: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE service_alert_events
         SET resolved_at = unixepoch(), updated_at = unixepoch(), resolution = ?
         WHERE monitor_id = ? AND resolved_at IS NULL",
    )
    .bind(resolution)
    .bind(id)
    .execute(connection)
    .await?;
    Ok(())
}

pub async fn purge(pool: &SqlitePool, retention_seconds: i64) -> Result<(), sqlx::Error> {
    if retention_seconds <= 0 {
        return Ok(());
    }
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    for query in [
        "DELETE FROM service_monitor_history WHERE observed_at < unixepoch() - ?",
        "DELETE FROM service_alert_events WHERE resolved_at < unixepoch() - ?",
        "DELETE FROM service_monitors WHERE enabled = 0 AND disabled_at < unixepoch() - ?",
    ] {
        sqlx::query(query)
            .bind(retention_seconds)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}
