use asterion_protocol::is_node_id;
use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize, Serializer};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use super::discovery::{self, Binding};
use super::input::{self, AssetInput, AssetPatch};
use crate::auth::{AdminSession, Csrf};
use crate::monitor::ApiError;

const SELECT_ASSET: &str = "SELECT a.*, n.name AS node_name, 'manual' AS source,
         (SELECT MAX(d.observed_at) FROM asset_bindings b JOIN discovered_resources d
          ON d.id = b.discovery_id WHERE b.asset_id = a.id) AS last_seen_at
     FROM service_assets a JOIN nodes n ON n.id = a.node_id";
const MAX_BINDINGS: i64 = 32;
const NOT_FOUND: ApiError =
    ApiError::new(StatusCode::NOT_FOUND, "asset_not_found", "找不到服务资产");
const DISCOVERY_NOT_FOUND: ApiError = ApiError::new(
    StatusCode::NOT_FOUND,
    "discovery_not_found",
    "找不到此节点上的发现资源",
);
const ASSET_CONFLICT: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "asset_binding_conflict",
    "请先解除资产现有绑定",
);

#[derive(FromRow, Serialize)]
pub(super) struct Asset {
    id: String,
    node_id: String,
    node_name: String,
    name: String,
    management: String,
    description: String,
    repo_url: Option<String>,
    deploy_path: Option<String>,
    config_path: Option<String>,
    #[serde(serialize_with = "array_json")]
    tags: String,
    notes: String,
    verified_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    last_seen_at: Option<i64>,
    source: String,
}

fn array_json<S: Serializer>(encoded: &str, serializer: S) -> Result<S::Ok, S::Error> {
    let tags = serde_json::from_str::<Vec<String>>(encoded).map_err(serde::ser::Error::custom)?;
    tags.serialize(serializer)
}

#[derive(Serialize)]
pub(super) struct AssetDetail {
    #[serde(flatten)]
    asset: Asset,
    bindings: Vec<Binding>,
}

#[derive(Serialize)]
pub(super) struct AssetList {
    assets: Vec<Asset>,
    has_more: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssetFilters {
    q: Option<String>,
    node_id: Option<String>,
    management: Option<String>,
    tag: Option<String>,
    limit: Option<u16>,
    offset: Option<u32>,
}

pub(super) async fn list(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    query: Result<Query<AssetFilters>, QueryRejection>,
) -> Result<Json<AssetList>, ApiError> {
    let Query(filters) = query.map_err(|_| ApiError::INVALID)?;
    let limit = filters.limit.unwrap_or(50);
    let offset = filters.offset.unwrap_or(0);
    let node_id = filters.node_id.as_deref();
    let management = filters.management.as_deref();
    if !(1..=100).contains(&limit)
        || offset > 100_000
        || node_id.is_some_and(|id| !is_node_id(id))
        || management.is_some_and(|label| !input::valid_management(label))
    {
        return Err(ApiError::INVALID);
    }
    let q = input::text(filters.q.as_deref().unwrap_or(""), 120, false)?;
    let tag = input::text(filters.tag.as_deref().unwrap_or(""), 32, false)?;

    let sql = format!(
        "{SELECT_ASSET} WHERE (? = '' OR instr(lower(a.name), lower(?)) > 0)
         AND (? IS NULL OR a.node_id = ?) AND (? IS NULL OR a.management = ?)
         AND (? = '' OR EXISTS(SELECT 1 FROM json_each(a.tags) WHERE lower(value) = lower(?)))
         ORDER BY a.updated_at DESC, a.id LIMIT ? OFFSET ?"
    );
    let mut assets = sqlx::query_as::<_, Asset>(&sql)
        .bind(&q)
        .bind(&q)
        .bind(node_id)
        .bind(node_id)
        .bind(management)
        .bind(management)
        .bind(&tag)
        .bind(&tag)
        .bind(i64::from(limit) + 1)
        .bind(offset)
        .fetch_all(&pool)
        .await?;
    let has_more = assets.len() > usize::from(limit);
    assets.truncate(usize::from(limit));
    Ok(Json(AssetList { assets, has_more }))
}

pub(super) async fn get(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<Json<AssetDetail>, ApiError> {
    let mut tx = pool.begin().await?;
    let result = detail(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(result))
}

pub(super) async fn create(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    body: Result<Json<AssetInput>, JsonRejection>,
) -> Result<(StatusCode, Json<AssetDetail>), ApiError> {
    let Json(body) = body.map_err(ApiError::invalid_json)?;
    let body = input::validate(body)?;
    let id = new_id()?;
    let mut tx = pool.begin().await?;
    exists_node(&mut tx, &body.node_id).await?;
    sqlx::query(
        "INSERT INTO service_assets
            (id, node_id, name, management, description, repo_url, deploy_path, config_path,
             tags, notes, verified_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CASE WHEN ? THEN unixepoch() END)",
    )
    .bind(&id)
    .bind(&body.node_id)
    .bind(&body.name)
    .bind(&body.management)
    .bind(&body.description)
    .bind(&body.repo_url)
    .bind(&body.deploy_path)
    .bind(&body.config_path)
    .bind(encode_tags(&body.tags)?)
    .bind(&body.notes)
    .bind(body.verified)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, &session.username, &id, "asset_create").await?;
    let result = detail(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(result)))
}

pub(super) async fn update(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    body: Result<Json<AssetPatch>, JsonRejection>,
) -> Result<Json<AssetDetail>, ApiError> {
    let Json(patch) = body.map_err(ApiError::invalid_json)?;
    let mut tx = pool.begin().await?;
    let current = require_asset(&mut tx, &id).await?;
    let node_changes = patch
        .node_id
        .as_ref()
        .is_some_and(|node| node != &current.node_id);
    let provenance_changes = node_changes
        || patch
            .management
            .as_ref()
            .is_some_and(|value| value != &current.management)
        || patch
            .deploy_path
            .as_ref()
            .is_some_and(|value| value != &current.deploy_path)
        || patch
            .config_path
            .as_ref()
            .is_some_and(|value| value != &current.config_path)
        || patch
            .repo_url
            .as_ref()
            .is_some_and(|value| value != &current.repo_url);
    let verified = patch
        .verified
        .or_else(|| provenance_changes.then_some(false));
    let body = input::validate(AssetInput {
        node_id: patch.node_id.unwrap_or(current.node_id),
        name: patch.name.unwrap_or(current.name),
        management: patch.management.unwrap_or(current.management),
        description: patch.description.unwrap_or(current.description),
        repo_url: patch.repo_url.unwrap_or(current.repo_url),
        deploy_path: patch.deploy_path.unwrap_or(current.deploy_path),
        config_path: patch.config_path.unwrap_or(current.config_path),
        tags: match patch.tags {
            Some(tags) => tags,
            None => serde_json::from_str(&current.tags).map_err(sqlx::Error::decode)?,
        },
        notes: patch.notes.unwrap_or(current.notes),
        verified: verified.unwrap_or_default(),
    })?;

    if node_changes {
        exists_node(&mut tx, &body.node_id).await?;
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM asset_bindings WHERE asset_id = ?)",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?
        {
            return Err(ASSET_CONFLICT);
        }
    }
    sqlx::query(
        "UPDATE service_assets SET node_id = ?, name = ?, management = ?, description = ?,
         repo_url = ?, deploy_path = ?, config_path = ?, tags = ?, notes = ?,
         verified_at = CASE ? WHEN 1 THEN unixepoch() WHEN 0 THEN NULL ELSE verified_at END,
         updated_at = unixepoch() WHERE id = ?",
    )
    .bind(&body.node_id)
    .bind(&body.name)
    .bind(&body.management)
    .bind(&body.description)
    .bind(&body.repo_url)
    .bind(&body.deploy_path)
    .bind(&body.config_path)
    .bind(encode_tags(&body.tags)?)
    .bind(&body.notes)
    .bind(verified)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    audit(&mut tx, &session.username, &id, "asset_update").await?;
    let result = detail(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(result))
}

pub(super) async fn remove(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND);
    }
    let mut tx = pool.begin().await?;
    let deleted = sqlx::query("DELETE FROM service_assets WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(NOT_FOUND);
    }
    audit(&mut tx, &session.username, &id, "asset_remove").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingInput {
    discovery_id: String,
}

pub(super) async fn bind(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
    body: Result<Json<BindingInput>, JsonRejection>,
) -> Result<Json<AssetDetail>, ApiError> {
    let Json(body) = body.map_err(ApiError::invalid_json)?;
    if !is_node_id(&body.discovery_id) {
        return Err(DISCOVERY_NOT_FOUND);
    }
    let mut tx = pool.begin().await?;
    let asset = require_asset(&mut tx, &id).await?;
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM discovered_resources WHERE id = ? AND node_id = ?)",
    )
    .bind(&body.discovery_id)
    .bind(&asset.node_id)
    .fetch_one(&mut *tx)
    .await?
    {
        return Err(DISCOVERY_NOT_FOUND);
    }

    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM asset_bindings WHERE asset_id = ? AND discovery_id = ?)",
    )
    .bind(&id)
    .bind(&body.discovery_id)
    .fetch_one(&mut *tx)
    .await?;
    if !existing {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM asset_bindings WHERE asset_id = ?")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_BINDINGS {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "asset_binding_limit",
                "资产绑定数量已达上限",
            ));
        }
        sqlx::query(
            "INSERT INTO asset_bindings (asset_id, discovery_id, node_id) VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(body.discovery_id)
        .bind(asset.node_id)
        .execute(&mut *tx)
        .await?;
        audit(&mut tx, &session.username, &id, "asset_bind").await?;
    }
    let result = detail(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(result))
}

pub(super) async fn unbind(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Path((id, discovery_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    if !is_node_id(&id) {
        return Err(NOT_FOUND);
    }
    if !is_node_id(&discovery_id) {
        return Err(DISCOVERY_NOT_FOUND);
    }
    let mut tx = pool.begin().await?;
    require_asset(&mut tx, &id).await?;
    let deleted = sqlx::query("DELETE FROM asset_bindings WHERE asset_id = ? AND discovery_id = ?")
        .bind(&id)
        .bind(discovery_id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() != 0 {
        audit(&mut tx, &session.username, &id, "asset_unbind").await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn detail(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<AssetDetail, ApiError> {
    let asset = require_asset(tx, id).await?;
    let bindings = discovery::bindings(tx, id).await?;
    Ok(AssetDetail { asset, bindings })
}

async fn require_asset(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<Asset, ApiError> {
    if !is_node_id(id) {
        return Err(NOT_FOUND);
    }
    let sql = format!("{SELECT_ASSET} WHERE a.id = ?");
    sqlx::query_as::<_, Asset>(&sql)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(NOT_FOUND)
}

async fn exists_node(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), ApiError> {
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM nodes WHERE id = ?)")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?
    {
        Ok(())
    } else {
        Err(ApiError::NODE_NOT_FOUND)
    }
}

async fn audit(
    tx: &mut Transaction<'_, Sqlite>,
    actor: &str,
    id: &str,
    action: &'static str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO asset_audit_events (actor, asset_id, action) VALUES (?, ?, ?)")
        .bind(actor)
        .bind(id)
        .bind(action)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn encode_tags(tags: &[String]) -> Result<String, ApiError> {
    Ok(serde_json::to_string(tags).map_err(sqlx::Error::decode)?)
}

fn new_id() -> Result<String, ApiError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "中心暂时无法生成资产标识",
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
