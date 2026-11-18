use asterion_protocol::{HEARTBEAT_TIMEOUT_SECS, NodeKind, is_node_id};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use super::error::ApiError;
use crate::model::Node;

const NODE_QUERY: &str = "SELECT n.*, CASE
    WHEN n.last_seen_at IS NULL THEN 'pending'
    WHEN n.last_seen_at > unixepoch() - ? AND EXISTS (
        SELECT 1 FROM agent_credentials a WHERE a.node_id = n.id AND a.revoked_at IS NULL
    ) THEN 'online'
    ELSE 'offline'
    END AS status FROM nodes n";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodeInput {
    pub name: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

impl NodeInput {
    pub fn validate(self) -> Result<Self, ApiError> {
        Ok(Self {
            name: validate_name(self.name)?,
            kind: self.kind,
            tags: validate_tags(self.tags)?,
            notes: validate_notes(self.notes)?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodePatch {
    pub name: Option<String>,
    pub kind: Option<NodeKind>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
}

impl NodePatch {
    pub fn validate(self) -> Result<Self, ApiError> {
        if self.name.is_none() && self.kind.is_none() && self.tags.is_none() && self.notes.is_none()
        {
            return Err(ApiError::INVALID_INPUT);
        }

        Ok(Self {
            name: self.name.map(validate_name).transpose()?,
            kind: self.kind,
            tags: self.tags.map(validate_tags).transpose()?,
            notes: self.notes.map(validate_notes).transpose()?,
        })
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

fn validate_tags(values: Vec<String>) -> Result<Vec<String>, ApiError> {
    if values.len() > 12 {
        return Err(ApiError::INVALID_INPUT);
    }
    let mut tags = Vec::with_capacity(values.len());
    for value in values {
        let tag = value.trim().to_owned();
        if !bounded_text(&tag, 32) {
            return Err(ApiError::INVALID_INPUT);
        }
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    Ok(tags)
}

pub(super) fn tags_json(tags: &[String]) -> Result<String, ApiError> {
    serde_json::to_string(tags).map_err(|_| ApiError::internal("tags_encode"))
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
    status: NodeStatus,
}

#[derive(Serialize)]
pub(super) struct NodeList {
    nodes: Vec<NodeView>,
    has_more: bool,
}

#[derive(FromRow)]
struct NodeRow {
    id: String,
    host_id: Option<String>,
    name: String,
    kind: String,
    tags: String,
    notes: String,
    os: Option<String>,
    arch: Option<String>,
    wsl_distro: Option<String>,
    agent_version: Option<String>,
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
        let tags =
            serde_json::from_str(&row.tags).map_err(|_| ApiError::internal("tags_decode"))?;
        Ok(Self {
            node: Node {
                id: row.id,
                host_id: row.host_id,
                name: row.name,
                kind,
                os: row.os,
                arch: row.arch,
                wsl_distro: row.wsl_distro,
                tags,
                notes: row.notes,
                agent_version: row.agent_version,
                last_seen_at: row.last_seen_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            status: row.status,
        })
    }
}

pub(super) async fn get(pool: &SqlitePool, id: &str) -> Result<NodeView, ApiError> {
    check_id(id)?;
    let sql = format!("{NODE_QUERY} WHERE n.id = ?");
    let node = sqlx::query_as::<_, NodeRow>(&sql)
        .bind(HEARTBEAT_TIMEOUT_SECS as i64)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    node.try_into()
}

#[derive(Default)]
pub(super) struct NodeFilters {
    pub q: Option<String>,
    pub tag: Option<String>,
    pub status: Option<NodeStatus>,
}

pub(super) async fn list(
    pool: &SqlitePool,
    limit: u16,
    offset: u32,
    filters: &NodeFilters,
) -> Result<NodeList, ApiError> {
    let sql = format!(
        "WITH node_status AS ({NODE_QUERY})
         SELECT * FROM node_status AS n
         WHERE (?2 IS NULL OR instr(lower(n.name), lower(?2)) > 0 OR instr(n.id, lower(?2)) > 0)
           AND (?3 IS NULL OR EXISTS (SELECT 1 FROM json_each(n.tags) AS t WHERE t.value = ?3))
           AND (?4 IS NULL OR n.status = ?4)
         ORDER BY n.created_at, n.id LIMIT ?5 OFFSET ?6"
    );
    let status = filters.status.as_ref().map(|status| match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Online => "online",
        NodeStatus::Offline => "offline",
    });
    let mut rows = sqlx::query_as::<_, NodeRow>(&sql)
        .bind(HEARTBEAT_TIMEOUT_SECS as i64)
        .bind(filters.q.as_deref())
        .bind(filters.tag.as_deref())
        .bind(status)
        .bind(i64::from(limit) + 1)
        .bind(i64::from(offset))
        .fetch_all(pool)
        .await?;
    let has_more = rows.len() > usize::from(limit);
    rows.truncate(usize::from(limit));
    Ok(NodeList {
        nodes: rows
            .into_iter()
            .map(NodeView::try_from)
            .collect::<Result<_, _>>()?,
        has_more,
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
