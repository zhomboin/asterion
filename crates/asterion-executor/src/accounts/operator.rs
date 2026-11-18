use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use asterion_protocol::operation::{AccountChange, valid_username};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ExecError, gateway, policy, run, time_budget};

use super::{
    Account, account, active_normal_user, files, groups, key_inventory, keys, supplementary_groups,
    users, valid_managed_group,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorPolicy {
    usernames: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
    backup_dir: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalPolicy {
    pub(crate) operator_socket: PathBuf,
    pub(crate) usernames: Vec<String>,
    #[serde(default)]
    pub(crate) groups: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Keys {
        username: String,
    },
    Change {
        change: AccountChange,
        expires_at: i64,
    },
}

impl LocalPolicy {
    pub(crate) fn validate(&self) -> Result<(), ExecError> {
        policy::validate_socket_path(&self.operator_socket)?;
        validate_allowlist(&self.usernames, &self.groups)
    }

    pub(crate) fn allows(&self, change: &AccountChange) -> bool {
        permits(&self.usernames, &self.groups, change)
    }
}

pub(crate) async fn remote_keys(address: &Path, username: &str) -> Result<Value, ExecError> {
    gateway::execute(
        address,
        &Request::Keys {
            username: username.to_owned(),
        },
        Instant::now() + Duration::from_secs(5),
    )
    .await
}

pub(crate) async fn execute(
    address: &Path,
    change: &AccountChange,
    expires_at: i64,
    deadline: Instant,
) -> Result<run::ResultData, ExecError> {
    gateway::execute(
        address,
        &Request::Change {
            change: change.clone(),
            expires_at,
        },
        deadline,
    )
    .await
}

pub async fn serve(address: &Path, file: &Path) -> Result<(), ExecError> {
    let policy: OperatorPolicy = policy::read_policy(file)?;
    // The privileged operator is distinct from the ordinary Agent account.
    if unsafe { libc::geteuid() } != 0 {
        return Err(ExecError::DENIED);
    }
    validate_allowlist(&policy.usernames, &policy.groups)?;
    files::validate_backup_dir(&policy.backup_dir)?;
    let policy = Arc::new(policy);
    gateway::serve(address, move |request: Request| {
        let policy = Arc::clone(&policy);
        async move {
            match request {
                Request::Keys { username } => {
                    if !policy.usernames.contains(&username) || !safe_user(&account(&username)?) {
                        return Err(ExecError::DENIED);
                    }
                    key_inventory(&username)
                }
                Request::Change { change, expires_at } => {
                    if !permits(&policy.usernames, &policy.groups, &change) {
                        return Err(ExecError::DENIED);
                    }
                    let result = apply(&policy, change, expires_at).await?;
                    serde_json::to_value(result).map_err(|_| ExecError::FAILED)
                }
            }
        }
    })
    .await
}

async fn apply(
    policy: &OperatorPolicy,
    change: AccountChange,
    expires_at: i64,
) -> Result<run::ResultData, ExecError> {
    let backup_dir = &policy.backup_dir;
    let started = Instant::now();
    let deadline = started + time_budget(expires_at)?.min(Duration::from_secs(25));
    match change {
        AccountChange::Create { username } => {
            let base = Path::new("/home");
            let metadata = std::fs::symlink_metadata(base).map_err(ExecError::io)?;
            if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
                return Err(ExecError::DENIED);
            }
            let home_present = match std::fs::symlink_metadata(base.join(&username)) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(ExecError::io(error)),
            };
            let known = users()?;
            if known.iter().any(|user| user.username == username) || home_present {
                return Err(ExecError::new("conflict", "用户名或家目录已存在"));
            }
            let uid = super::next_uid(&known)?;
            run::command(
                Path::new("/usr/sbin/useradd"),
                &[
                    "--uid".into(),
                    uid.to_string(),
                    "--user-group".into(),
                    "--create-home".into(),
                    "--base-dir".into(),
                    "/home".into(),
                    "--shell".into(),
                    "/bin/bash".into(),
                    "--".into(),
                    username,
                ],
                deadline,
            )
            .await
        }
        AccountChange::SetGroups {
            username,
            groups: selected,
        } => {
            let user = account(&username)?;
            if !safe_user(&user) {
                return Err(ExecError::DENIED);
            }
            let local_groups = groups()?;
            if selected.iter().any(|name| {
                !local_groups.iter().any(|group| {
                    group.name == *name && group.gid != user.gid && valid_managed_group(group)
                })
            }) {
                return Err(ExecError::DENIED);
            }
            let original = supplementary_groups(&user, &local_groups);
            let mut updated: BTreeSet<_> = original.keys().cloned().collect();
            // Only the explicitly allowed subset of memberships changes.
            let managed: BTreeSet<_> = selected.iter().cloned().collect();
            updated.retain(|name| !policy.groups.contains(name) || managed.contains(name));
            updated.extend(managed);
            if updated.iter().any(|name| !valid_username(name)) {
                return Err(ExecError::DENIED);
            }
            if updated.iter().eq(original.keys()) {
                return Ok(run::ResultData::empty(started));
            }
            let snapshot = serde_json::to_vec(&original.keys().collect::<Vec<_>>())
                .map_err(|_| ExecError::FAILED)?;
            files::write_backup(backup_dir, &username, "groups", &snapshot)?;
            run::command(
                Path::new("/usr/sbin/usermod"),
                &[
                    "--groups".into(),
                    updated.into_iter().collect::<Vec<_>>().join(","),
                    "--".into(),
                    username,
                ],
                deadline,
            )
            .await
        }
        AccountChange::AddKey { username, key } => {
            let user = account(&username)?;
            if !safe_user(&user) {
                return Err(ExecError::DENIED);
            }
            let parsed = keys::parse(&key, false).ok_or(ExecError::INVALID)?;
            let existing =
                files::AuthorizedKeys::read(&user, true, true)?.ok_or(ExecError::FAILED)?;
            if contains_key(&existing.bytes, &parsed.fingerprint) {
                return Err(ExecError::new("conflict", "该公钥指纹已经存在"));
            }
            let mut bytes = existing.bytes.clone();
            if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(key.trim().as_bytes());
            bytes.push(b'\n');
            existing.replace(&user, &bytes, backup_dir)?;
            Ok(run::ResultData::empty(started))
        }
        AccountChange::RemoveKey {
            username,
            fingerprint,
        } => {
            let user = account(&username)?;
            if !safe_user(&user) {
                return Err(ExecError::DENIED);
            }
            let existing =
                files::AuthorizedKeys::read(&user, true, false)?.ok_or(ExecError::FAILED)?;
            let mut result = Vec::with_capacity(existing.bytes.len());
            for line in existing.bytes.split_inclusive(|byte| *byte == b'\n') {
                if !matches_key(line, &fingerprint) {
                    result.extend_from_slice(line);
                }
            }
            if result.len() == existing.bytes.len() {
                return Err(ExecError::new("not_found", "该公钥指纹不存在"));
            }
            existing.replace(&user, &result, backup_dir)?;
            Ok(run::ResultData::empty(started))
        }
    }
}

fn safe_user(user: &Account) -> bool {
    active_normal_user(user)
        && user.gid >= 1000
        && user.home == format!("/home/{}", user.username)
        && groups().is_ok_and(|groups| {
            groups.iter().all(|group| {
                (!group.members.contains(&user.username) && group.gid != user.gid)
                    || !super::protected_group(&group.name)
            })
        })
}

fn validate_allowlist(usernames: &[String], groups: &[String]) -> Result<(), ExecError> {
    if usernames.is_empty()
        || usernames.len() > 64
        || groups.len() > 32
        || usernames
            .iter()
            .any(|name| name == "root" || !valid_username(name))
        || groups
            .iter()
            .any(|name| !valid_username(name) || super::protected_group(name))
    {
        return Err(ExecError::POLICY);
    }
    Ok(())
}

fn permits(users: &[String], groups: &[String], change: &AccountChange) -> bool {
    change.valid()
        && users.iter().any(|username| username == change.username())
        && match change {
            AccountChange::SetGroups {
                groups: selected, ..
            } => selected.iter().all(|group| groups.contains(group)),
            _ => true,
        }
}

fn contains_key(file: &[u8], fingerprint: &str) -> bool {
    file.split(|byte| *byte == b'\n')
        .any(|line| matches_key(line, fingerprint))
}

fn matches_key(line: &[u8], fingerprint: &str) -> bool {
    std::str::from_utf8(line)
        .ok()
        .and_then(|line| keys::parse(line, true))
        .is_some_and(|key| key.fingerprint == fingerprint)
}
