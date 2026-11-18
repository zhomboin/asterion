use asterion_protocol::is_node_id;
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::auth::{AdminSession, Csrf};
use crate::monitor::ApiError;

const NOT_FOUND: ApiError = ApiError::new(
    StatusCode::NOT_FOUND,
    "alert_rule_not_found",
    "找不到告警规则",
);
const DUPLICATE: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "alert_rule_exists",
    "该节点已有此指标的告警规则",
);
const SELECT_RULE: &str = "SELECT r.id, r.node_id, n.name AS node_name, r.metric, r.threshold,
    r.for_seconds, r.enabled, r.created_at, r.updated_at
    FROM alert_rules r JOIN nodes n ON n.id = r.node_id";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Filter {
    node_id: Option<String>,
    limit: Option<u16>,
    offset: Option<u32>,
    state: Option<String>,
}

impl Filter {
    fn pagination(&self) -> Result<(u16, u32), ApiError> {
        let limit = self.limit.unwrap_or(50);
        let offset = self.offset.unwrap_or(0);
        if !(1..=100).contains(&limit)
            || offset > 100_000
            || self.node_id.as_deref().is_some_and(|id| !is_node_id(id))
        {
            return Err(ApiError::INVALID);
        }
        Ok((limit, offset))
    }
}

#[derive(FromRow, Serialize)]
pub(super) struct Rule {
    id: String,
    node_id: String,
    node_name: String,
    metric: String,
    threshold: Option<f64>,
    for_seconds: i64,
    enabled: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize)]
pub(super) struct Rules {
    rules: Vec<Rule>,
    has_more: bool,
}

pub(super) async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    query: Result<Query<Filter>, QueryRejection>,
) -> Result<Json<Rules>, ApiError> {
    let Query(filter) = query.map_err(|_| ApiError::INVALID)?;
    let (limit, offset) = filter.pagination()?;
    if filter.state.is_some() {
        return Err(ApiError::INVALID);
    }
    let query = format!(
        "{SELECT_RULE} WHERE (? IS NULL OR r.node_id = ?)
        ORDER BY r.created_at DESC, r.id LIMIT ? OFFSET ?"
    );
    let mut rules = sqlx::query_as::<_, Rule>(&query)
        .bind(&filter.node_id)
        .bind(&filter.node_id)
        .bind(i64::from(limit) + 1)
        .bind(offset)
        .fetch_all(&pool)
        .await?;
    let has_more = rules.len() > usize::from(limit);
    rules.truncate(usize::from(limit));
    Ok(Json(Rules { rules, has_more }))
}

#[derive(FromRow, Serialize)]
struct Event {
    id: String,
    rule_id: String,
    node_id: String,
    node_name: String,
    metric: String,
    status: String,
    value: Option<f64>,
    threshold: Option<f64>,
    opened_at: i64,
    updated_at: i64,
    resolved_at: Option<i64>,
}

#[derive(Serialize)]
pub(super) struct Events {
    alerts: Vec<Event>,
    has_more: bool,
}

pub(super) async fn events(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    query: Result<Query<Filter>, QueryRejection>,
) -> Result<Json<Events>, ApiError> {
    let Query(filter) = query.map_err(|_| ApiError::INVALID)?;
    let (limit, offset) = filter.pagination()?;
    let state = filter.state.as_deref().unwrap_or("active");
    if !matches!(state, "active" | "resolved" | "all") {
        return Err(ApiError::INVALID);
    }
    let mut alerts = sqlx::query_as::<_, Event>(
        "SELECT *, CASE WHEN resolved_at IS NULL THEN 'active' ELSE 'resolved' END AS status
         FROM alert_events WHERE (? IS NULL OR node_id = ?)
         AND (? = 'all' OR (? = 'active' AND resolved_at IS NULL) OR (? = 'resolved' AND resolved_at IS NOT NULL))
         ORDER BY opened_at DESC, id LIMIT ? OFFSET ?",
    )
    .bind(&filter.node_id)
    .bind(&filter.node_id)
    .bind(state)
    .bind(state)
    .bind(state)
    .bind(i64::from(limit) + 1)
    .bind(offset)
    .fetch_all(&pool)
    .await?;
    let has_more = alerts.len() > usize::from(limit);
    alerts.truncate(usize::from(limit));
    Ok(Json(Events { alerts, has_more }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleDraft {
    node_id: String,
    metric: String,
    threshold: Option<f64>,
    for_seconds: i64,
    enabled: bool,
}

impl RuleDraft {
    fn validate(&self) -> Result<(), ApiError> {
        let condition = match self.metric.as_str() {
            "offline" => self.threshold.is_none(),
            "cpu" | "memory" | "disk" => self
                .threshold
                .is_some_and(|value| (0.0..=100.0).contains(&value)),
            _ => false,
        };
        if !condition || !is_node_id(&self.node_id) || !(0..=3600).contains(&self.for_seconds) {
            return Err(ApiError::INVALID);
        }
        Ok(())
    }
}

pub(super) async fn create(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    body: Result<Json<RuleDraft>, JsonRejection>,
) -> Result<(StatusCode, Json<Rule>), ApiError> {
    let Json(draft) = body.map_err(ApiError::invalid_json)?;
    draft.validate()?;
    let mut tx = pool.begin().await?;
    ensure_node(&mut tx, &draft.node_id).await?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO alert_rules (id, node_id, metric, threshold, for_seconds, enabled)
         VALUES (lower(hex(randomblob(16))), ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&draft.node_id)
    .bind(&draft.metric)
    .bind(draft.threshold)
    .bind(draft.for_seconds)
    .bind(draft.enabled)
    .fetch_one(&mut *tx)
    .await
    .map_err(rule_error)?;
    let rule = read_rule(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(rule)))
}

pub(super) async fn update(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    body: Result<Json<RuleDraft>, JsonRejection>,
) -> Result<Json<Rule>, ApiError> {
    let Json(draft) = body.map_err(ApiError::invalid_json)?;
    draft.validate()?;
    let mut tx = pool.begin().await?;
    let before = read_rule(&mut tx, &id).await?;
    if before.node_id != draft.node_id {
        ensure_node(&mut tx, &draft.node_id).await?;
    }
    let changed = before.node_id != draft.node_id
        || before.metric != draft.metric
        || before.threshold != draft.threshold
        || before.for_seconds != draft.for_seconds
        || before.enabled != draft.enabled;
    if changed {
        sqlx::query(
            "UPDATE alert_rules SET node_id = ?, metric = ?, threshold = ?, for_seconds = ?, enabled = ?,
             breach_since = NULL, last_sample_at = NULL, updated_at = unixepoch() WHERE id = ?",
        )
        .bind(&draft.node_id)
        .bind(&draft.metric)
        .bind(draft.threshold)
        .bind(draft.for_seconds)
        .bind(draft.enabled)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(rule_error)?;
        sqlx::query(
            "UPDATE alert_events SET resolved_at = unixepoch(), updated_at = unixepoch()
             WHERE rule_id = ? AND resolved_at IS NULL",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    }

    let rule = read_rule(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(rule))
}

pub(super) async fn remove(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND);
    }
    let deleted = sqlx::query("DELETE FROM alert_rules WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn read_rule(connection: &mut SqliteConnection, id: &str) -> Result<Rule, ApiError> {
    if !is_node_id(id) {
        return Err(NOT_FOUND);
    }
    let query = format!("{SELECT_RULE} WHERE r.id = ?");
    sqlx::query_as::<_, Rule>(&query)
        .bind(id)
        .fetch_optional(connection)
        .await?
        .ok_or(NOT_FOUND)
}

async fn ensure_node(connection: &mut SqliteConnection, id: &str) -> Result<(), ApiError> {
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(id)
        .fetch_one(connection)
        .await?
    {
        Ok(())
    } else {
        Err(ApiError::NODE_NOT_FOUND)
    }
}

fn rule_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        DUPLICATE
    } else {
        error.into()
    }
}
