use asterion_protocol::is_node_id;
use serde::{Deserialize, Deserializer};

use crate::monitor::ApiError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssetInput {
    pub node_id: String,
    pub name: String,
    pub management: String,
    #[serde(default)]
    pub description: String,
    pub repo_url: Option<String>,
    pub deploy_path: Option<String>,
    pub config_path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AssetPatch {
    pub node_id: Option<String>,
    pub name: Option<String>,
    pub management: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub repo_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub deploy_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub config_path: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub verified: Option<bool>,
}

fn nullable<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::deserialize(deserializer).map(Some)
}

pub(super) fn validate(mut input: AssetInput) -> Result<AssetInput, ApiError> {
    if !is_node_id(&input.node_id) || !valid_management(&input.management) || input.tags.len() > 12
    {
        return Err(ApiError::INVALID);
    }
    input.name = text(&input.name, 120, false)?;
    if input.name.is_empty() {
        return Err(ApiError::INVALID);
    }
    input.description = text(&input.description, 1500, true)?;
    input.notes = text(&input.notes, 1500, true)?;
    input.deploy_path = optional(input.deploy_path, 1024)?;
    input.config_path = optional(input.config_path, 1024)?;
    input.repo_url = optional(input.repo_url, 1024)?;
    if input
        .repo_url
        .as_deref()
        .is_some_and(|value| !valid_repo_url(value))
    {
        return Err(ApiError::INVALID);
    }
    let mut tags = Vec::with_capacity(input.tags.len());
    for value in input.tags {
        let tag = text(&value, 32, false)?;
        if tag.is_empty() {
            return Err(ApiError::INVALID);
        }
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    input.tags = tags;
    Ok(input)
}

pub(super) fn valid_management(value: &str) -> bool {
    matches!(
        value,
        "docker" | "compose" | "systemd" | "process" | "other"
    )
}

pub(super) fn text(value: &str, max_chars: usize, multiline: bool) -> Result<String, ApiError> {
    let value = value.trim();
    if value.chars().count() > max_chars
        || value
            .chars()
            .any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t')))
    {
        return Err(ApiError::INVALID);
    }
    Ok(value.to_owned())
}

fn optional(value: Option<String>, max_chars: usize) -> Result<Option<String>, ApiError> {
    Ok(value
        .map(|value| text(&value, max_chars, false))
        .transpose()?
        .filter(|value| !value.is_empty()))
}

fn valid_repo_url(value: &str) -> bool {
    if value.chars().any(char::is_whitespace) {
        return false;
    }
    if let Some((scheme, remainder)) = value.split_once("://") {
        let host = remainder.split('/').next().unwrap_or("");
        return matches!(scheme, "https" | "http" | "ssh" | "git")
            && !host.is_empty()
            && (!matches!(scheme, "https" | "http") || !host.contains('@'));
    }
    value
        .split_once(':')
        .is_some_and(|(host, path)| host.starts_with("git@") && !path.is_empty())
}
