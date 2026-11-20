use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::queries::{bus_error, is_container_id};
use super::{ApiError, ensure_node};
use crate::auth::{AdminSession, Csrf};
use crate::ssh_bus::SshBus;

const PAGE_SIZE: usize = 12;
const CHANGED: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "resource_changed",
    "采集对象已变化，请重新发现并选择",
);

pub(super) fn routes() -> Router<SqlitePool> {
    Router::new()
        .route(
            "/api/v1/nodes/{id}/managed-resources/{kind}",
            get(list).post(add),
        )
        .route(
            "/api/v1/nodes/{id}/managed-resources/{kind}/live",
            get(live),
        )
        .route(
            "/api/v1/nodes/{id}/managed-resources/{kind}/{resource}",
            delete(remove),
        )
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Process,
    Docker,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Docker => "docker",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Process {
        pid: u32,
        start_ticks: u64,
    },
    Docker {
        container_id: String,
        engine_id: String,
    },
}

#[derive(Serialize, sqlx::FromRow)]
struct Resource {
    id: i64,
    target: String,
    name: String,
    engine_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    #[serde(default)]
    offset: u32,
}

#[derive(Serialize)]
struct Page {
    resources: Vec<Resource>,
    has_more: bool,
}

async fn page(pool: &SqlitePool, id: &str, kind: Kind, offset: u32) -> Result<Page, ApiError> {
    ensure_node(pool, id).await?;
    let mut resources: Vec<Resource> = sqlx::query_as(
        "SELECT id, target, name, engine_id FROM managed_resources
         WHERE node_id = ? AND kind = ? ORDER BY id LIMIT ? OFFSET ?",
    )
    .bind(id)
    .bind(kind.key())
    .bind((PAGE_SIZE + 1) as i64)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let has_more = resources.len() > PAGE_SIZE;
    resources.truncate(PAGE_SIZE);
    Ok(Page {
        resources,
        has_more,
    })
}

async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path((id, kind)): Path<(String, Kind)>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Page>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    Ok(Json(page(&pool, &id, kind, query.offset).await?))
}

async fn live(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<SshBus>>,
    Path((id, kind)): Path<(String, Kind)>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::INVALID)?;
    let page = page(&pool, &id, kind, query.offset).await?;
    let targets: Vec<_> = page.resources.iter().map(|row| &row.target).collect();
    let mut result = match kind {
        _ if targets.is_empty() => Value::Null,
        Kind::Process => bus
            .query(
                &id,
                "processes",
                json!({"names": targets, "sort": "pid", "limit": 13}),
            )
            .await
            .map_err(bus_error)?,
        Kind::Docker => bus
            .query(&id, "docker", json!({"ids": targets}))
            .await
            .map_err(bus_error)?,
    };
    filter_live_result(kind, &page.resources, &mut result);
    Ok(Json(result))
}

fn filter_live_result(kind: Kind, resources: &[Resource], result: &mut Value) {
    match kind {
        Kind::Process => {
            if let Some(processes) = result.as_array_mut() {
                processes.retain(|row| {
                    let name = row["name"].as_str();
                    resources
                        .iter()
                        .any(|resource| name == Some(resource.target.as_str()))
                });
            }
        }
        Kind::Docker => {
            let engine = result["engine_id"].as_str().unwrap_or_default().to_owned();
            if let Some(containers) = result.get_mut("containers").and_then(Value::as_array_mut) {
                containers.retain(|row| {
                    let id = row["id"].as_str();
                    resources.iter().any(|resource| {
                        resource.engine_id == engine && id == Some(resource.target.as_str())
                    })
                });
            }
        }
    }
}

async fn add(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<SshBus>>,
    Path((id, kind)): Path<(String, Kind)>,
    body: Result<Json<Target>, JsonRejection>,
) -> Result<Json<Resource>, ApiError> {
    let Json(body) = body.map_err(ApiError::invalid_json)?;
    ensure_node(&pool, &id).await?;
    let (target, name, engine_id) = match (kind, body) {
        (Kind::Process, Target::Process { pid, start_ticks })
            if (1..=i32::MAX as u32).contains(&pid) =>
        {
            let value = bus
                .query(&id, "process", json!({"pid": pid}))
                .await
                .map_err(bus_error)?;
            if value["pid"].as_u64() != Some(u64::from(pid))
                || value["start_ticks"].as_u64() != Some(start_ticks)
            {
                return Err(CHANGED);
            }
            let name = checked_name(&value["name"])?;
            (name.clone(), name, String::new())
        }
        (
            Kind::Docker,
            Target::Docker {
                container_id,
                engine_id,
            },
        ) if is_container_id(&container_id) && valid_text(&engine_id, 128) => {
            let value = bus
                .query(&id, "docker", json!({"ids": [&container_id]}))
                .await
                .map_err(bus_error)?;
            if value["status"] != "available" || value["engine_id"].as_str() != Some(&engine_id) {
                return Err(CHANGED);
            }
            let row = value["containers"]
                .as_array()
                .and_then(|containers| {
                    containers
                        .iter()
                        .find(|row| row["id"].as_str() == Some(&container_id))
                })
                .ok_or(CHANGED)?;
            let name = checked_name(&row["name"]).unwrap_or_else(|_| container_id.clone());
            (container_id, name, engine_id)
        }
        _ => return Err(ApiError::INVALID),
    };
    let resource = sqlx::query_as(
        "INSERT INTO managed_resources (node_id, kind, target, name, engine_id)
         VALUES (?, ?, ?, ?, ?) ON CONFLICT (node_id, kind, engine_id, target)
         DO UPDATE SET name = excluded.name RETURNING id, target, name, engine_id",
    )
    .bind(id)
    .bind(kind.key())
    .bind(target)
    .bind(name)
    .bind(engine_id)
    .fetch_one(&pool)
    .await?;
    Ok(Json(resource))
}

async fn remove(
    _: Csrf,
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path((id, kind, resource)): Path<(String, Kind, i64)>,
) -> Result<StatusCode, ApiError> {
    ensure_node(&pool, &id).await?;
    sqlx::query("DELETE FROM managed_resources WHERE node_id = ? AND kind = ? AND id = ?")
        .bind(id)
        .bind(kind.key())
        .bind(resource)
        .execute(&pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn checked_name(value: &Value) -> Result<String, ApiError> {
    value
        .as_str()
        .filter(|name| valid_text(name, 256))
        .map(str::to_owned)
        .ok_or(CHANGED)
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(target: &str, engine_id: &str) -> Resource {
        Resource {
            id: 1,
            target: target.into(),
            name: target.into(),
            engine_id: engine_id.into(),
        }
    }

    #[test]
    fn live_docker_requires_matching_engine_and_container_id() {
        let resources = [resource("first", "native"), resource("second", "desktop")];
        let mut report = json!({
            "status": "available", "engine_id": "desktop", "containers": [
                { "id": "first" }, { "id": "second" }, { "id": "other" }
            ]
        });
        filter_live_result(Kind::Docker, &resources, &mut report);
        assert_eq!(report["engine_id"], "desktop");
        assert_eq!(report["containers"], json!([{ "id": "second" }]));
    }

    #[test]
    fn live_processes_only_returns_managed_names() {
        let mut report = json!([{ "name": "app" }, { "name": "app-worker" }, { "pid": 1 }]);
        filter_live_result(Kind::Process, &[resource("app", "")], &mut report);
        assert_eq!(report, json!([{ "name": "app" }]));
    }
}
