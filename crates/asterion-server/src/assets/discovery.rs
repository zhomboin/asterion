use std::borrow::Cow;

use asterion_protocol::is_node_id;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::auth::AdminSession;
use crate::monitor::ApiError;

const MAX_UNBOUND_PER_NODE: i64 = 2048;
const MAX_OBSERVATIONS: usize = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscoveryFilters {
    kind: Option<String>,
    limit: Option<u16>,
    offset: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct DiscoveryList {
    discoveries: Vec<Discovery>,
    has_more: bool,
}

#[derive(FromRow, Serialize)]
pub(super) struct Discovery {
    id: String,
    kind: String,
    resource_id: String,
    resource_scope: String,
    name: String,
    #[serde(serialize_with = "metadata_json")]
    metadata: String,
    observed_at: i64,
    source: String,
}

#[derive(FromRow, Serialize)]
pub(super) struct Binding {
    #[sqlx(flatten)]
    #[serde(flatten)]
    discovery: Discovery,
    bound_at: i64,
}

fn metadata_json<S: Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error> {
    let value = serde_json::from_str::<Value>(text).map_err(serde::ser::Error::custom)?;
    value.serialize(serializer)
}

pub(super) async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(node_id): Path<String>,
    query: Result<Query<DiscoveryFilters>, QueryRejection>,
) -> Result<Json<DiscoveryList>, ApiError> {
    let Query(filters) = query.map_err(|_| ApiError::INVALID)?;
    let limit = filters.limit.unwrap_or(50);
    let offset = filters.offset.unwrap_or(0);
    let kind = filters.kind.as_deref();
    if !(1..=100).contains(&limit)
        || offset > 100_000
        || kind.is_some_and(|kind| !matches!(kind, "docker" | "process" | "systemd"))
    {
        return Err(ApiError::INVALID);
    }
    if !is_node_id(&node_id)
        || !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
            .bind(&node_id)
            .fetch_one(&pool)
            .await?
    {
        return Err(ApiError::NODE_NOT_FOUND);
    }

    let mut discoveries = sqlx::query_as::<_, Discovery>(
        "SELECT id, kind, resource_id, resource_scope, name, metadata, observed_at, 'agent' AS source
         FROM discovered_resources WHERE node_id = ? AND (? IS NULL OR kind = ?)
         ORDER BY observed_at DESC, id LIMIT ? OFFSET ?",
    )
    .bind(node_id)
    .bind(kind)
    .bind(kind)
    .bind(i64::from(limit) + 1)
    .bind(offset)
    .fetch_all(&pool)
    .await?;
    let has_more = discoveries.len() > usize::from(limit);
    discoveries.truncate(usize::from(limit));
    Ok(Json(DiscoveryList {
        discoveries,
        has_more,
    }))
}

pub(super) async fn bindings(
    connection: &mut SqliteConnection,
    asset_id: &str,
) -> Result<Vec<Binding>, ApiError> {
    Ok(sqlx::query_as::<_, Binding>(
        "SELECT d.id, d.kind, d.resource_id, d.resource_scope, d.name, d.metadata, d.observed_at,
         'agent' AS source, b.bound_at FROM asset_bindings b
         JOIN discovered_resources d ON d.id = b.discovery_id
         WHERE b.asset_id = ? ORDER BY b.bound_at, d.id",
    )
    .bind(asset_id)
    .fetch_all(connection)
    .await?)
}

pub(crate) async fn observe(
    pool: &SqlitePool,
    node_id: &str,
    query_kind: &str,
    result: &Value,
) -> Result<(), ApiError> {
    let observations = project(query_kind, result);
    if observations.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for entry in observations {
        sqlx::query(
            "INSERT INTO discovered_resources
                (id, node_id, kind, resource_scope, resource_id, name, metadata, observed_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, unixepoch() WHERE EXISTS (SELECT 1 FROM nodes WHERE id = ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, metadata = excluded.metadata,
                 observed_at = excluded.observed_at",
        )
        .bind(resource_id(node_id, &entry))
        .bind(node_id)
        .bind(entry.kind)
        .bind(entry.scope)
        .bind(entry.resource)
        .bind(entry.name)
        .bind(entry.metadata.to_string())
        .bind(node_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "DELETE FROM discovered_resources WHERE id IN (
             SELECT d.id FROM discovered_resources d WHERE d.node_id = ?
             AND NOT EXISTS (SELECT 1 FROM asset_bindings b WHERE b.discovery_id = d.id)
             ORDER BY d.observed_at DESC, d.id LIMIT -1 OFFSET ?
         )",
    )
    .bind(node_id)
    .bind(MAX_UNBOUND_PER_NODE)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

struct Observation<'a> {
    kind: &'static str,
    scope: Cow<'a, str>,
    resource: Cow<'a, str>,
    name: &'a str,
    metadata: Value,
}

fn resource_id(node: &str, observation: &Observation<'_>) -> String {
    let mut hash = Sha256::new();
    for component in [
        node,
        observation.kind,
        &observation.scope,
        &observation.resource,
    ] {
        hash.update(component.as_bytes());
        hash.update([0]);
    }
    hash.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn project<'a>(query_kind: &str, result: &'a Value) -> Vec<Observation<'a>> {
    let list = |value: &'a Value| {
        value
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .take(MAX_OBSERVATIONS)
    };
    match query_kind {
        "docker" if result.get("status").and_then(Value::as_str) == Some("available") => {
            let engine = field(result, "engine_id", 256);
            let endpoint = field(result, "endpoint_fingerprint", 256);
            let scope = if let Some(id) = engine {
                format!("engine:{id}")
            } else if let Some(fingerprint) = endpoint {
                format!("endpoint:{fingerprint}")
            } else {
                return Vec::new();
            };

            result
                .get("containers")
                .into_iter()
                .flat_map(list)
                .filter_map(|row| {
                    let id = field(row, "id", 64)?;
                    let name = field(row, "name", 256)?;
                    Some(Observation {
                        kind: "docker",
                        scope: Cow::Owned(scope.clone()),
                        resource: Cow::Borrowed(id),
                        name,
                        metadata: json!({
                            "engine_id": engine,
                            "endpoint_fingerprint": endpoint,
                            "image": field(row, "image", 512),
                            "state": field(row, "state", 64),
                            "compose_project": field(row, "compose_project", 256),
                            "compose_service": field(row, "compose_service", 256),
                        }),
                    })
                })
                .collect()
        }
        "processes" => list(result).filter_map(process).collect(),
        "process" => process(result).into_iter().collect(),
        "systemd" => result
            .get("units")
            .into_iter()
            .flat_map(list)
            .filter_map(unit)
            .collect(),
        "systemd_service" => unit(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn process(row: &Value) -> Option<Observation<'_>> {
    let name = field(row, "name", 512)?;
    let pid = row.get("pid")?.as_u64()?;
    let start_ticks = row.get("start_ticks")?.as_u64()?;
    Some(Observation {
        kind: "process",
        scope: Cow::Borrowed("procfs"),
        resource: Cow::Owned(format!("{pid}:{start_ticks}")),
        name,
        metadata: json!({
            "pid": pid,
            "start_ticks": start_ticks,
            "started_at": row.get("started_at").and_then(Value::as_i64),
            "state": field(row, "state", 64),
        }),
    })
}

fn unit(row: &Value) -> Option<Observation<'_>> {
    let name = field(row, "name", 255)?;
    Some(Observation {
        kind: "systemd",
        scope: Cow::Borrowed("systemd"),
        resource: Cow::Borrowed(name),
        name,
        metadata: json!({
            "active": field(row, "active", 64),
            "sub": field(row, "sub", 64),
            "description": field(row, "description", 512),
            "working_directory": field(row, "working_directory", 4096),
            "fragment_path": field(row, "fragment_path", 4096),
        }),
    })
}

fn field<'a>(value: &'a Value, key: &str, max_bytes: usize) -> Option<&'a str> {
    value.get(key)?.as_str().filter(|value| {
        !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
    })
}
