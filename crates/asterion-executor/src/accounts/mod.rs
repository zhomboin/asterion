use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use asterion_protocol::operation::valid_username;
use serde::Serialize;
use serde_json::{Value, json};

use crate::ExecError;

mod files;
mod keys;
pub(crate) mod operator;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_USERS: usize = 256;
const MAX_INVENTORY_BYTES: usize = 140 * 1024;

#[derive(Serialize)]
pub(crate) struct Account {
    pub(crate) username: String,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) shell: String,
    pub(crate) home: String,
    pub(crate) groups: Vec<String>,
    pub(crate) system: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct Group {
    name: String,
    gid: u32,
    members: BTreeSet<String>,
}

pub(crate) fn inventory() -> Result<Value, ExecError> {
    let mut users = users()?;
    users.sort_unstable_by(|first, second| {
        first
            .uid
            .cmp(&second.uid)
            .then_with(|| first.username.cmp(&second.username))
    });
    let mut rows = Vec::new();
    let mut used = 0;
    let mut truncated = users.len() > MAX_USERS;
    for user in users.into_iter().take(MAX_USERS) {
        let row = serde_json::to_value(user).map_err(|_| ExecError::FAILED)?;
        let size = serde_json::to_vec(&row)
            .map_err(|_| ExecError::FAILED)?
            .len();
        if used + size + 1 > MAX_INVENTORY_BYTES {
            truncated = true;
            break;
        }
        used += size + 1;
        rows.push(row);
    }
    Ok(json!({"users":rows,"truncated":truncated}))
}

pub(crate) fn users() -> Result<Vec<Account>, ExecError> {
    let passwd = read_text("/etc/passwd")?;
    let groups = groups()?;
    let uid_min = read_text("/etc/login.defs")
        .ok()
        .and_then(|contents| configured_number(&contents, "UID_MIN"))
        .unwrap_or(1000);
    Ok(passwd
        .lines()
        .filter_map(|line| parse_user(line, &groups, uid_min))
        .collect())
}

pub(crate) fn groups() -> Result<Vec<Group>, ExecError> {
    let group = read_text("/etc/group")?;
    Ok(group.lines().filter_map(parse_group).collect())
}

fn read_text(path: &str) -> Result<String, ExecError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(ExecError::io)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ExecError::io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ExecError::FAILED);
    }
    String::from_utf8(bytes).map_err(|_| ExecError::FAILED)
}

fn configured_number(contents: &str, key: &str) -> Option<u32> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next()? == key).then(|| fields.next()?.parse().ok())?
    })
}

fn parse_user(line: &str, groups: &[Group], uid_min: u32) -> Option<Account> {
    let mut fields = line.split(':');
    let username = fields.next()?.to_owned();
    if username.len() > 128 || username.is_empty() || username.chars().any(char::is_control) {
        return None;
    }
    fields.next()?;
    let uid = fields.next()?.parse::<u32>().ok()?;
    let gid = fields.next()?.parse::<u32>().ok()?;
    fields.next()?;
    let home = fields.next()?.to_owned();
    let shell = fields.next()?.to_owned();
    if home.len() > 1024 || shell.len() > 256 || fields.next().is_some() {
        return None;
    }
    let members = groups
        .iter()
        .filter(|group| group.gid == gid || group.members.contains(&username))
        .map(|group| group.name.clone())
        .take(128)
        .collect();
    Some(Account {
        system: uid < uid_min || uid == 65_534,
        username,
        uid,
        gid,
        shell,
        home,
        groups: members,
    })
}

fn parse_group(line: &str) -> Option<Group> {
    let mut fields = line.split(':');
    let name = fields.next()?.to_owned();
    if name.len() > 128 || name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    fields.next()?;
    let gid = fields.next()?.parse().ok()?;
    let members = fields
        .next()?
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    Some(Group { name, gid, members })
}

pub(crate) fn account(username: &str) -> Result<Account, ExecError> {
    if !valid_username(username) {
        return Err(ExecError::INVALID);
    }
    users()?
        .into_iter()
        .find(|user| user.username == username)
        .ok_or(ExecError::new("not_found", "Linux 用户不存在"))
}

pub(crate) fn key_inventory(username: &str) -> Result<Value, ExecError> {
    let user = account(username)?;
    let current = files::AuthorizedKeys::read(&user, false, false)?;
    let data = current
        .as_ref()
        .map_or(b"".as_slice(), |file| file.bytes.as_slice());
    keys::inventory(data)
}

fn active_normal_user(user: &Account) -> bool {
    !user.system
        && (1000..65_534).contains(&user.uid)
        && user.username != "root"
        && Path::new(&user.home).is_absolute()
}

fn valid_managed_group(group: &Group) -> bool {
    (1000..65_534).contains(&group.gid)
        && valid_username(&group.name)
        && !protected_group(&group.name)
}

fn protected_group(name: &str) -> bool {
    matches!(
        name,
        "root"
            | "sudo"
            | "wheel"
            | "adm"
            | "docker"
            | "lxd"
            | "libvirt"
            | "staff"
            | "disk"
            | "shadow"
            | "systemd-journal"
            | "kvm"
            | "microk8s"
            | "incus-admin"
            | "podman"
            | "operator"
            | "daemon"
            | "tty"
    )
}

pub(crate) fn next_uid(existing: &[Account]) -> Result<u32, ExecError> {
    let login = read_text("/etc/login.defs")?;
    let used: BTreeSet<_> = existing.iter().map(|user| user.uid).collect();
    let minimum = configured_number(&login, "UID_MIN")
        .unwrap_or(1000)
        .max(1000);
    let maximum = configured_number(&login, "UID_MAX")
        .unwrap_or(60_000)
        .min(65_533);
    let gid_minimum = configured_number(&login, "GID_MIN").unwrap_or(1000);
    if gid_minimum < 1000 {
        return Err(ExecError::DENIED);
    }
    (minimum..=maximum)
        .find(|uid| !used.contains(uid))
        .ok_or(ExecError::DENIED)
}

fn supplementary_groups(user: &Account, groups: &[Group]) -> BTreeMap<String, bool> {
    groups
        .iter()
        .filter(|group| group.gid != user.gid && group.members.contains(&user.username))
        .map(|group| (group.name.clone(), valid_managed_group(group)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_inventory_marks_system_accounts_and_group_membership() {
        let groups: Vec<_> = ["main:x:1001:", "team:x:1002:example"]
            .into_iter()
            .filter_map(parse_group)
            .collect();
        let user = parse_user(
            "example:x:1001:1001::/home/example:/bin/bash",
            &groups,
            1000,
        )
        .unwrap();
        assert!(!user.system);
        assert_eq!(user.groups, ["main", "team"]);
        let daemon = parse_user("d:x:2:2::/var/empty:/sbin/nologin", &groups, 1000).unwrap();
        assert!(daemon.system);
        assert!(!active_normal_user(&daemon));
    }
}
