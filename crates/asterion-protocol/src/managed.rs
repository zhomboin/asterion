use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedVerb {
    Install,
    SaveReload,
    Create,
    Recreate,
    Start,
    Stop,
    Restart,
}

impl ManagedVerb {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Install => "packages",
            Self::SaveReload => "configs",
            _ => "containers",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAction {
    pub key: String,
    pub verb: ManagedVerb,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ContainerSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerSettings {
    pub restart: String,
    pub ports: Vec<[u16; 2]>,
    pub mounts: Vec<String>,
}

impl ManagedAction {
    pub fn valid(&self) -> bool {
        self.settings.as_ref().is_none_or(|value| {
            matches!(self.verb, ManagedVerb::Create | ManagedVerb::Recreate)
                && matches!(
                    value.restart.as_str(),
                    "no" | "always" | "on-failure" | "unless-stopped"
                )
                && value.ports.len() <= 16
                && value.ports.iter().flatten().all(|port| *port != 0)
                && value.mounts.len() <= 16
                && value
                    .mounts
                    .iter()
                    .all(|name| super::operation::valid_identifier(name, 64))
        }) && super::operation::valid_identifier(&self.key, 64)
            && super::is_lower_hex(&self.revision, 64)
            && match self.verb {
                ManagedVerb::SaveReload => {
                    self.profile.is_none()
                        && self
                            .digest
                            .as_deref()
                            .is_some_and(|digest| super::is_lower_hex(digest, 64))
                }
                ManagedVerb::Create | ManagedVerb::Recreate => {
                    self.digest.is_none()
                        && self
                            .profile
                            .as_deref()
                            .is_some_and(|profile| super::operation::valid_identifier(profile, 64))
                }
                _ => self.profile.is_none() && self.digest.is_none(),
            }
    }
}
