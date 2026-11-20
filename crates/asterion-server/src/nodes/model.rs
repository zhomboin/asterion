use asterion_protocol::{NodeKind, is_node_id};
use serde::{Deserialize, Deserializer, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use super::error::ApiError;
use crate::model::Node;

const NODE_QUERY: &str = "SELECT n.*,
    (SELECT name FROM hosts WHERE id = n.host_id) AS host_name,
    CASE
    WHEN n.ssh_online_until IS NULL THEN 'pending'
    WHEN n.ssh_online_until > unixepoch() THEN 'online'
    ELSE 'offline'
    END AS status FROM nodes n";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodeInput {
    pub name: String,
    pub ssh_profile: String,
    pub ssh_auth: Option<super::ssh_auth::Input>,
    pub host_name: Option<String>,
    pub kind: NodeKind,
    #[serde(default)]
    pub notes: String,
}

impl NodeInput {
    pub fn validate(self, local: bool) -> Result<Self, ApiError> {
        if self.ssh_auth.is_some() && !(local && self.ssh_profile.starts_with("ssh://")) {
            return Err(ApiError::INVALID_INPUT);
        }
        Ok(Self {
            name: validate_name(self.name)?,
            ssh_profile: profile(self.ssh_profile, local)?,
            ssh_auth: self.ssh_auth,
            host_name: self.host_name.map(validate_name).transpose()?,
            kind: self.kind,
            notes: validate_notes(self.notes)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodePatch {
    pub name: Option<String>,
    pub ssh_profile: Option<String>,
    pub ssh_auth: Option<super::ssh_auth::Input>,
    #[serde(default, deserialize_with = "nullable_name")]
    pub host_name: Option<Option<String>>,
    pub kind: Option<NodeKind>,
    pub notes: Option<String>,
}

impl NodePatch {
    pub fn validate(self, local: bool) -> Result<Self, ApiError> {
        if self.ssh_auth.is_some() && !local {
            return Err(ApiError::INVALID_INPUT);
        }
        if self.ssh_profile.is_none()
            && self.ssh_auth.is_none()
            && self.name.is_none()
            && self.host_name.is_none()
            && self.kind.is_none()
            && self.notes.is_none()
        {
            return Err(ApiError::INVALID_INPUT);
        }

        Ok(Self {
            ssh_auth: self.ssh_auth,
            ssh_profile: self
                .ssh_profile
                .map(|name| profile(name, local))
                .transpose()?,
            name: self.name.map(validate_name).transpose()?,
            host_name: self
                .host_name
                .map(|name| name.map(validate_name).transpose())
                .transpose()?,
            kind: self.kind,
            notes: self.notes.map(validate_notes).transpose()?,
        })
    }
}

fn nullable_name<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::deserialize(deserializer).map(Some)
}

fn profile(value: String, local: bool) -> Result<String, ApiError> {
    if crate::ssh::valid_connection(&value, local) {
        Ok(value)
    } else {
        Err(ApiError::INVALID_INPUT)
    }
}

fn validate_name(value: String) -> Result<String, ApiError> {
    let value = value.trim();
    if !bounded_text(value, 100) {
        return Err(ApiError::INVALID_INPUT);
    }
    Ok(value.to_owned())
}

fn validate_notes(value: String) -> Result<String, ApiError> {
    let value = value.trim();
    if value.chars().count() > 1000
        || value
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return Err(ApiError::INVALID_INPUT);
    }
    Ok(value.to_owned())
}

pub(super) fn bounded_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

#[derive(Debug, Deserialize, Serialize, sqlx::Type, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub(super) enum NodeStatus {
    Pending,
    Online,
    Offline,
}

#[derive(Serialize)]
pub(super) struct NodeView {
    #[serde(flatten)]
    node: Node,
    host_name: Option<String>,
    ssh_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssh_auth: Option<super::ssh_auth::View>,
    ssh_error: Option<String>,
    status: NodeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<NodeMetrics>,
}

#[derive(Serialize)]
struct NodeMetrics {
    observed_at: i64,
    cpu_percent: Option<f64>,
    memory_percent: Option<f64>,
    disk_percent: Option<f64>,
}

#[derive(Serialize)]
pub(super) struct NodeList {
    nodes: Vec<NodeView>,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_counts: Option<NodeCounts>,
}

#[derive(FromRow, Serialize)]
struct NodeCounts {
    pending: i64,
    online: i64,
    offline: i64,
}

#[derive(FromRow)]
struct NodeRow {
    id: String,
    host_id: Option<String>,
    host_name: Option<String>,
    name: String,
    kind: String,
    notes: String,
    os: Option<String>,
    arch: Option<String>,
    wsl_distro: Option<String>,
    ssh_profile: Option<String>,
    ssh_error: Option<String>,
    last_seen_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    status: NodeStatus,
}

impl TryFrom<NodeRow> for NodeView {
    type Error = ApiError;

    fn try_from(row: NodeRow) -> Result<Self, Self::Error> {
        let kind = match row.kind.as_str() {
            "linux" => NodeKind::Linux,
            "wsl" => NodeKind::Wsl,
            _ => return Err(ApiError::internal("node_kind")),
        };
        Ok(Self {
            node: Node {
                id: row.id,
                host_id: row.host_id,
                name: row.name,
                kind,
                os: row.os,
                arch: row.arch,
                wsl_distro: row.wsl_distro,
                notes: row.notes,
                last_seen_at: row.last_seen_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            host_name: row.host_name,
            ssh_profile: row.ssh_profile,
            ssh_auth: None,
            ssh_error: row.ssh_error,
            status: row.status,
            metrics: None,
        })
    }
}

pub(super) async fn get(pool: &SqlitePool, id: &str) -> Result<NodeView, ApiError> {
    check_id(id)?;
    let sql = format!("{NODE_QUERY} WHERE n.id = ?");
    let node = sqlx::query_as::<_, NodeRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let mut view = NodeView::try_from(node)?;
    view.ssh_auth = super::ssh_auth::view(pool, id).await?;
    Ok(view)
}

#[derive(Default)]
pub(super) struct NodeFilters {
    pub q: Option<String>,
    pub status: Option<NodeStatus>,
    pub with_metrics: bool,
}

#[derive(FromRow)]
struct NodeListRow {
    #[sqlx(flatten)]
    node: NodeRow,
    metric_observed_at: Option<i64>,
    metric_sample: Option<String>,
}

impl TryFrom<NodeListRow> for NodeView {
    type Error = ApiError;

    fn try_from(row: NodeListRow) -> Result<Self, Self::Error> {
        let mut view = Self::try_from(row.node)?;
        if let Some((observed_at, sample)) = row.metric_observed_at.zip(row.metric_sample) {
            let sample = serde_json::from_str(&sample)
                .map_err(|_| ApiError::internal("node_metric_decode"))?;
            let statistics = crate::trends::statistics(&sample);
            view.metrics = Some(NodeMetrics {
                observed_at,
                cpu_percent: statistics.cpu,
                memory_percent: statistics.memory,
                disk_percent: statistics.disk,
            });
        }
        Ok(view)
    }
}

pub(super) async fn list(
    pool: &SqlitePool,
    limit: u16,
    offset: u32,
    filters: &NodeFilters,
) -> Result<NodeList, ApiError> {
    let sql = format!(
        "WITH node_status AS ({NODE_QUERY}), node_page AS (
             SELECT * FROM node_status AS n
             WHERE (?1 IS NULL OR instr(lower(n.name), lower(?1)) > 0 OR instr(n.id, lower(?1)) > 0)
               AND (?2 IS NULL OR n.status = ?2)
             ORDER BY n.created_at, n.id LIMIT ?3 OFFSET ?4
         )
         SELECT p.*, m.observed_at AS metric_observed_at, m.sample AS metric_sample
         FROM node_page AS p
         LEFT JOIN metric_samples AS m ON ?5 AND m.node_id = p.id AND m.observed_at = (
             SELECT max(observed_at) FROM metric_samples
             WHERE node_id = p.id AND observed_at <= unixepoch()
         )
         ORDER BY p.created_at, p.id"
    );
    let status = filters.status.as_ref().map(|status| match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Online => "online",
        NodeStatus::Offline => "offline",
    });
    let mut tx = pool.begin().await?;
    let mut rows = sqlx::query_as::<_, NodeListRow>(&sql)
        .bind(filters.q.as_deref())
        .bind(status)
        .bind(i64::from(limit) + 1)
        .bind(i64::from(offset))
        .bind(filters.with_metrics)
        .fetch_all(&mut *tx)
        .await?;
    let status_counts = if filters.with_metrics {
        let counts_sql = format!(
            "SELECT count(*) FILTER (WHERE status = 'pending') AS pending,
                    count(*) FILTER (WHERE status = 'online') AS online,
                    count(*) FILTER (WHERE status = 'offline') AS offline
             FROM ({NODE_QUERY})"
        );
        Some(sqlx::query_as(&counts_sql).fetch_one(&mut *tx).await?)
    } else {
        None
    };
    tx.commit().await?;
    let has_more = rows.len() > usize::from(limit);
    rows.truncate(usize::from(limit));
    Ok(NodeList {
        nodes: rows
            .into_iter()
            .map(NodeView::try_from)
            .collect::<Result<_, _>>()?,
        has_more,
        status_counts,
    })
}

pub(super) fn check_id(id: &str) -> Result<(), ApiError> {
    if is_node_id(id) {
        Ok(())
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub(super) async fn audit(
    tx: &mut Transaction<'_, Sqlite>,
    actor: &str,
    node_id: &str,
    action: &'static str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO node_audit_events (actor, node_id, action) VALUES (?, ?, ?)")
        .bind(actor)
        .bind(node_id)
        .bind(action)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
