use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::ApiError;
use super::metrics::ensure_node;
use crate::agent_bus::{AgentBus, BusError};
use crate::assets::observe;
use crate::auth::AdminSession;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProcessQuery {
    q: Option<String>,
    sort: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

pub(super) async fn processes(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(id): Path<String>,
    options: Result<Query<ProcessQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(options) = options.map_err(|_| ApiError::INVALID)?;
    let q = options.q.as_deref().unwrap_or("").trim();
    let sort = options.sort.as_deref().unwrap_or("cpu");
    let limit = options.limit.unwrap_or(100);
    let offset = options.offset.unwrap_or(0);
    if q.chars().count() > 100
        || q.chars().any(char::is_control)
        || !matches!(sort, "cpu" | "memory" | "pid" | "name")
        || !(1..=256).contains(&limit)
        || offset > 10_000
    {
        return Err(ApiError::INVALID);
    }
    run_query(
        &pool,
        &bus,
        &id,
        "processes",
        json!({"q": q, "sort": sort, "limit": limit, "offset": offset}),
    )
    .await
}

pub(super) async fn process(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((id, pid)): Path<(String, u32)>,
) -> Result<Json<Value>, ApiError> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(ApiError::INVALID);
    }
    run_query(&pool, &bus, &id, "process", json!({"pid": pid})).await
}

pub(super) async fn docker(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(id): Path<String>,
    options: Result<Query<UnitQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(options) = options.map_err(|_| ApiError::INVALID)?;
    let args = match options.q {
        Some(q) if q.len() <= 256 && !q.chars().any(char::is_control) => json!({"q": q.trim()}),
        Some(_) => return Err(ApiError::INVALID),
        None => json!({}),
    };
    run_query(&pool, &bus, &id, "docker", args).await
}

pub(super) async fn docker_stats(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((id, container)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !is_container_id(&container) {
        return Err(ApiError::INVALID);
    }
    run_query(&pool, &bus, &id, "docker_stats", json!({"id": container})).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogsQuery {
    lines: Option<u16>,
}

pub(super) async fn docker_logs(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((id, container)): Path<(String, String)>,
    query: Result<Query<LogsQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let lines = query.lines.unwrap_or(100);
    if !is_container_id(&container) || !(1..=200).contains(&lines) {
        return Err(ApiError::INVALID);
    }
    run_query(
        &pool,
        &bus,
        &id,
        "docker_logs",
        json!({"id": container, "lines": lines}),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnitQuery {
    q: Option<String>,
}

pub(super) async fn systemd(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(id): Path<String>,
    query: Result<Query<UnitQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let search = query.q.as_deref().unwrap_or("").trim();
    if search.len() > 128 || search.chars().any(char::is_control) {
        return Err(ApiError::INVALID);
    }
    run_query(&pool, &bus, &id, "systemd", json!({"q":search})).await
}

pub(super) async fn systemd_service(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path((id, unit)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if unit.starts_with('-')
        || unit.len() > 255
        || !unit.ends_with(".service")
        || !unit.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@' | b'\\' | b':')
        })
    {
        return Err(ApiError::INVALID);
    }
    run_query(&pool, &bus, &id, "systemd_service", json!({"unit": unit})).await
}

async fn run_query(
    pool: &SqlitePool,
    bus: &AgentBus,
    id: &str,
    kind: &str,
    args: Value,
) -> Result<Json<Value>, ApiError> {
    ensure_node(pool, id).await?;
    let result = bus.query(id, kind, args).await.map_err(bus_error)?;
    observe(pool, id, kind, &result).await?;
    Ok(Json(result))
}

pub(super) fn is_container_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(crate) fn bus_error(error: BusError) -> ApiError {
    match error {
        BusError::Offline => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_offline",
            "Agent 当前未连接",
        ),
        BusError::Busy => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_busy",
            "Agent 查询繁忙，请稍后重试",
        ),
        BusError::Timeout => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "agent_timeout",
            "等待 Agent 查询超时",
        ),
        BusError::Unsupported => ApiError::new(
            StatusCode::CONFLICT,
            "agent_upgrade_required",
            "请升级 Agent 后查询",
        ),
        BusError::Rejected { code, .. } => match &*code {
            "permission_denied" => ApiError::new(
                StatusCode::FORBIDDEN,
                "agent_permission_denied",
                "Agent 没有读取权限",
            ),
            "not_found" => ApiError::new(
                StatusCode::NOT_FOUND,
                "agent_resource_not_found",
                "节点上已找不到此资源",
            ),
            "invalid_input" | "invalid_argument" => ApiError::INVALID,
            "unavailable" | "unreachable" | "not_installed" | "disabled" => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "agent_resource_unavailable",
                "节点上的资源当前不可用",
            ),
            "timeout" => {
                ApiError::new(StatusCode::GATEWAY_TIMEOUT, "agent_timeout", "节点读取超时")
            }
            _ => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "agent_query_failed",
                "节点未能完成查询",
            ),
        },
    }
}
