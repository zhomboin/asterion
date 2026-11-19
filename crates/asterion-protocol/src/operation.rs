use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceVerb {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSignal {
    Term,
    Kill,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationAction {
    Managed {
        change: crate::managed::ManagedAction,
    },
    Systemd {
        unit: String,
        verb: ServiceVerb,
    },
    Docker {
        engine_id: String,
        container_id: String,
        verb: ServiceVerb,
    },
    Process {
        pid: u32,
        start_ticks: u64,
        signal: ProcessSignal,
    },
    Command {
        name: String,
        parameters: BTreeMap<String, String>,
    },
    Account {
        change: AccountChange,
    },
}

impl OperationAction {
    pub fn valid(&self) -> bool {
        match self {
            Self::Managed { change } => change.valid(),
            Self::Systemd { unit, .. } => valid_service_unit(unit),
            Self::Docker {
                engine_id,
                container_id,
                ..
            } => valid_identifier(engine_id, 128) && crate::is_lower_hex(container_id, 64),
            Self::Process {
                pid, start_ticks, ..
            } => *pid > 1 && *pid <= i32::MAX as u32 && *start_ticks > 0,
            Self::Account { change } => change.valid(),
            Self::Command { name, parameters } => {
                valid_identifier(name, 64)
                    && parameters.len() <= 12
                    && parameters.iter().all(|(key, value)| {
                        valid_identifier(key, 64)
                            && value.len() <= 256
                            && !value.chars().any(char::is_control)
                    })
            }
        }
    }

    pub fn privileged(&self) -> bool {
        matches!(
            self,
            Self::Process {
                signal: ProcessSignal::Kill,
                ..
            } | Self::Account { .. }
                | Self::Managed { .. }
        )
    }

    pub fn target(&self) -> String {
        match self {
            Self::Managed { change } => change.key.clone(),
            Self::Systemd { unit, .. } => unit.clone(),
            Self::Docker {
                engine_id,
                container_id,
                ..
            } => format!("{engine_id}/{container_id}"),
            Self::Process {
                pid, start_ticks, ..
            } => format!("{pid}/{start_ticks}"),
            Self::Command { name, .. } => name.clone(),
            Self::Account { change } => change.username().to_owned(),
        }
    }
}

pub fn valid_service_unit(unit: &str) -> bool {
    let Some(name) = unit.strip_suffix(".service") else {
        return false;
    };
    if name.is_empty() || unit.len() > 255 || name.starts_with(['-', '@']) || name.ends_with('@') {
        return false;
    }

    let mut bytes = name.bytes();
    let mut instance = false;
    while let Some(byte) = bytes.next() {
        match byte {
            b'@' if !instance => instance = true,
            b'\\'
                if bytes.next() == Some(b'x')
                    && bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit())
                    && bytes.next().is_some_and(|byte| byte.is_ascii_hexdigit()) => {}
            b'.' | b'_' | b'-' | b':' => {}
            byte if byte.is_ascii_alphanumeric() => {}
            _ => return false,
        }
    }
    true
}

pub fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'@' | b':')
        })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationTask {
    pub task_id: String,
    pub action: OperationAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountChange {
    Create {
        username: String,
    },
    SetGroups {
        username: String,
        groups: Vec<String>,
    },
    AddKey {
        username: String,
        key: String,
    },
    RemoveKey {
        username: String,
        fingerprint: String,
    },
}

impl AccountChange {
    pub fn username(&self) -> &str {
        match self {
            Self::Create { username }
            | Self::SetGroups { username, .. }
            | Self::AddKey { username, .. }
            | Self::RemoveKey { username, .. } => username,
        }
    }

    pub fn valid(&self) -> bool {
        valid_username(self.username())
            && self.username() != "root"
            && match self {
                Self::Create { .. } => true,
                Self::SetGroups { groups, .. } => {
                    groups.len() <= 8 && groups.iter().all(|name| valid_username(name))
                }
                Self::AddKey { key, .. } => {
                    !key.is_empty() && key.len() <= 8192 && !key.chars().any(char::is_control)
                }
                Self::RemoveKey { fingerprint, .. } => {
                    fingerprint.starts_with("SHA256:")
                        && fingerprint.len() == 50
                        && fingerprint[7..]
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
                }
            }
    }
}

pub fn valid_username(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=32).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::valid_service_unit;

    #[test]
    fn accepts_concrete_service_names_with_systemd_escapes() {
        assert!(valid_service_unit("example.service"));
        assert!(valid_service_unit(r"example@foo\x2dbar.service"));
        assert!(valid_service_unit(&format!("{}.service", "a".repeat(247))));
        assert!(!valid_service_unit(&format!("{}.service", "a".repeat(248))));
        for invalid in [
            ".service",
            "-example.service",
            "@foo.service",
            "example@.service",
            "example@foo@bar.service",
            "example*.service",
            "example.socket",
            r"example@\xy2.service",
            r"example@\x2.service",
        ] {
            assert!(!valid_service_unit(invalid), "{invalid}");
        }
    }
}
