use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use asterion_protocol::operation::{valid_identifier, valid_service_unit};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::ExecError;

const MAX_POLICY_BYTES: u64 = 64 * 1024;
const MAX_RULES: usize = 96;
const MAX_COMMANDS: usize = 24;
const MAX_ARGS: usize = 24;
const MAX_TIMEOUT_SECONDS: u64 = 120;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Policy {
    pub(crate) systemd_units: Vec<String>,
    pub(crate) docker: Option<DockerPolicy>,
    pub(crate) accounts: Option<crate::accounts::operator::LocalPolicy>,
    pub(crate) process: ProcessPolicy,
    pub(crate) commands: Vec<CommandPolicy>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DockerPolicy {
    pub(crate) operator_socket: PathBuf,
    pub(crate) targets: Vec<DockerTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DockerTarget {
    pub(crate) engine_id: String,
    pub(crate) container_ids: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ProcessPolicy {
    pub(crate) term: bool,
    pub(crate) kill: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandPolicy {
    pub(crate) name: String,
    pub(crate) program: PathBuf,
    #[serde(default)]
    pub(crate) args: Vec<Argument>,
    #[serde(default)]
    pub(crate) parameters: BTreeMap<String, Rule>,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_seconds: u64,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Argument {
    Fixed(String),
    Parameter(ParameterRef),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParameterRef {
    parameter: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Rule {
    Integer { min: u32, max: u32 },
    Choice { values: Vec<String> },
    Identifier { max_length: usize },
}

impl Policy {
    pub(crate) fn from_file(path: &Path) -> Result<Self, ExecError> {
        let mut policy: Self = read_policy(path)?;
        if policy.systemd_units.len() > MAX_RULES
            || policy.commands.len() > MAX_COMMANDS
            || policy
                .systemd_units
                .iter()
                .any(|unit| !valid_service_unit(unit))
        {
            return Err(ExecError::POLICY);
        }
        if let Some(docker) = &policy.docker {
            validate_socket_path(&docker.operator_socket)?;
            validate_targets(&docker.targets)?;
        }

        if let Some(accounts) = &policy.accounts {
            accounts.validate()?;
        }

        let mut names = std::collections::BTreeSet::new();
        for command in &mut policy.commands {
            command.validate()?;
            if !names.insert(command.name.clone()) {
                return Err(ExecError::POLICY);
            }
        }
        Ok(policy)
    }

    pub(crate) fn enabled(&self) -> bool {
        !self.systemd_units.is_empty()
            || self.docker.is_some()
            || self.accounts.is_some()
            || self.process.term
            || self.process.kill
            || !self.commands.is_empty()
    }

    pub(crate) fn catalog(&self) -> Value {
        let targets: Vec<_> = self
            .docker
            .as_ref()
            .into_iter()
            .flat_map(|docker| &docker.targets)
            .map(
                |target| json!({"engine_id":target.engine_id,"container_ids":target.container_ids}),
            )
            .collect();
        let engine_ids: Vec<_> = self
            .docker
            .as_ref()
            .into_iter()
            .flat_map(|docker| &docker.targets)
            .map(|target| &target.engine_id)
            .collect();

        let commands: Vec<_> = self
            .commands
            .iter()
            .map(|command| {
                json!({
                    "name": command.name,
                    "timeout_seconds": command.timeout_seconds,
                    "parameters": command.parameters.iter()
                        .map(|(name, rule)| rule.describe(name))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({
            "enabled": self.enabled(),
            "systemd_units": self.systemd_units,
            "docker_engine_ids": engine_ids,
            "docker_targets": targets,
            "accounts": {
                "enabled": self.accounts.is_some(),
                "usernames": self.accounts.as_ref().map(|policy| policy.usernames.as_slice()).unwrap_or_default(),
                "groups": self.accounts.as_ref().map(|policy| policy.groups.as_slice()).unwrap_or_default(),
            },
            "process": {"term": self.process.term, "kill": self.process.kill},
            "commands": commands,
        })
    }
}

impl CommandPolicy {
    fn validate(&mut self) -> Result<(), ExecError> {
        if !valid_identifier(&self.name, 64)
            || self.args.len() > MAX_ARGS
            || self.parameters.len() > 12
            || !(1..=MAX_TIMEOUT_SECONDS).contains(&self.timeout_seconds)
            || self
                .parameters
                .iter()
                .any(|(name, rule)| !valid_identifier(name, 64) || !rule.valid())
        {
            return Err(ExecError::POLICY);
        }
        self.program = validate_program(&self.program)?;
        let mut used = std::collections::BTreeSet::new();
        for arg in &self.args {
            match arg {
                Argument::Fixed(value) if !safe_argument(value) => return Err(ExecError::POLICY),
                Argument::Fixed(_) => {}
                Argument::Parameter(reference)
                    if self.parameters.contains_key(&reference.parameter) =>
                {
                    used.insert(&reference.parameter);
                }
                Argument::Parameter(_) => return Err(ExecError::POLICY),
            }
        }
        if used.len() != self.parameters.len() {
            return Err(ExecError::POLICY);
        }
        Ok(())
    }

    pub(crate) fn arguments(
        &self,
        submitted: &BTreeMap<String, String>,
    ) -> Result<Vec<String>, ExecError> {
        if submitted.len() != self.parameters.len() {
            return Err(ExecError::INVALID);
        }
        for (name, rule) in &self.parameters {
            if !submitted.get(name).is_some_and(|value| rule.matches(value)) {
                return Err(ExecError::INVALID);
            }
        }
        Ok(self
            .args
            .iter()
            .map(|argument| match argument {
                Argument::Fixed(value) => value.clone(),
                Argument::Parameter(reference) => submitted[&reference.parameter].clone(),
            })
            .collect())
    }
}

impl Rule {
    fn valid(&self) -> bool {
        match self {
            Self::Integer { min, max } => min <= max,
            Self::Choice { values } => {
                !values.is_empty()
                    && values.len() <= 24
                    && values.iter().all(|value| safe_parameter(value))
            }
            Self::Identifier { max_length } => (1..=128).contains(max_length),
        }
    }

    fn matches(&self, value: &str) -> bool {
        if !safe_parameter(value) {
            return false;
        }
        match self {
            Self::Integer { min, max } => {
                value.bytes().all(|byte| byte.is_ascii_digit())
                    && value
                        .parse::<u32>()
                        .is_ok_and(|number| (*min..=*max).contains(&number))
            }
            Self::Choice { values } => values.iter().any(|allowed| value == allowed),
            Self::Identifier { max_length } => valid_identifier(value, *max_length),
        }
    }

    fn describe(&self, name: &str) -> Value {
        match self {
            Self::Integer { min, max } => json!({"name":name,"type":"integer","min":min,"max":max}),
            Self::Choice { values } => json!({"name":name,"type":"choice","values":values}),
            Self::Identifier { max_length } => {
                json!({"name":name,"type":"identifier","max_length":max_length})
            }
        }
    }
}

pub(crate) fn validate_targets(targets: &[DockerTarget]) -> Result<(), ExecError> {
    if targets.is_empty() || targets.len() > MAX_RULES {
        return Err(ExecError::POLICY);
    }
    let mut engines = std::collections::BTreeSet::new();
    for target in targets {
        if !valid_identifier(&target.engine_id, 128)
            || !engines.insert(&target.engine_id)
            || target.container_ids.is_empty()
            || target.container_ids.len() > MAX_RULES
            || target
                .container_ids
                .iter()
                .any(|id| !asterion_protocol::is_lower_hex(id, 64))
        {
            return Err(ExecError::POLICY);
        }
    }
    Ok(())
}

pub(crate) fn allowed_container(targets: &[DockerTarget], engine: &str, id: &str) -> bool {
    targets.iter().any(|target| {
        target.engine_id == engine && target.container_ids.iter().any(|allowed| allowed == id)
    })
}

pub(crate) fn validate_socket_path(path: &Path) -> Result<(), ExecError> {
    if !path.is_absolute()
        || path.as_os_str().len() > 100
        || !path
            .to_str()
            .is_some_and(|value| !value.chars().any(char::is_control))
    {
        return Err(ExecError::POLICY);
    }
    Ok(())
}

pub(crate) fn read_policy<T: DeserializeOwned>(path: &Path) -> Result<T, ExecError> {
    if !path.is_absolute() {
        return Err(ExecError::POLICY);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| ExecError::POLICY)?;
    let stat = file.metadata().map_err(|_| ExecError::POLICY)?;
    if !stat.is_file() || !trusted_owner(&stat) || stat.mode() & 0o022 != 0 {
        return Err(ExecError::POLICY);
    }
    let mut bytes = Vec::new();
    file.take(MAX_POLICY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ExecError::POLICY)?;
    if bytes.len() as u64 > MAX_POLICY_BYTES {
        return Err(ExecError::POLICY);
    }
    serde_json::from_slice(&bytes).map_err(|_| ExecError::POLICY)
}

fn validate_program(path: &Path) -> Result<PathBuf, ExecError> {
    if !path.is_absolute() {
        return Err(ExecError::POLICY);
    }
    let program = path.canonicalize().map_err(|_| ExecError::POLICY)?;
    const DISPATCHERS: &[&str] = &[
        "sh",
        "bash",
        "dash",
        "zsh",
        "ksh",
        "fish",
        "env",
        "sudo",
        "su",
        "pkexec",
        "busybox",
        "python",
        "perl",
        "ruby",
        "node",
        "php",
        "lua",
        "powershell",
        "pwsh",
    ];
    let dispatcher = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| DISPATCHERS.contains(&name) || name.starts_with("python"))
    };
    if dispatcher(path) || dispatcher(&program) {
        return Err(ExecError::POLICY);
    }
    let file = program.metadata().map_err(|_| ExecError::POLICY)?;
    if !file.is_file() || file.mode() & 0o111 == 0 || file.mode() & 0o6000 != 0 {
        return Err(ExecError::POLICY);
    }
    for part in program.ancestors() {
        let metadata = part.metadata().map_err(|_| ExecError::POLICY)?;
        if !trusted_owner(&metadata) || metadata.mode() & 0o022 != 0 {
            return Err(ExecError::POLICY);
        }
    }
    Ok(program)
}

fn trusted_owner(metadata: &std::fs::Metadata) -> bool {
    // geteuid only inspects the credentials of this process.
    metadata.uid() == 0 || metadata.uid() == unsafe { libc::geteuid() }
}

fn safe_argument(value: &str) -> bool {
    value.len() <= 256 && !value.chars().any(char::is_control)
}

fn safe_parameter(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && safe_argument(value)
}

fn default_timeout() -> u64 {
    15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_values_never_become_unapproved_arguments() {
        let command: CommandPolicy = serde_json::from_value(json!({
            "name":"example",
            "program":"/usr/bin/printf",
            "args":["%s",{"parameter":"name"}],
            "parameters":{"name":{"type":"identifier","max_length":12}}
        }))
        .unwrap();
        let mut params = BTreeMap::from([("name".into(), "safe.name".into())]);
        assert_eq!(command.arguments(&params).unwrap(), ["%s", "safe.name"]);
        for bad in ["--version", "../test", "a\nb", "way-too-long_name"] {
            params.insert("name".into(), bad.into());
            assert!(command.arguments(&params).is_err());
        }
        assert!(validate_program(Path::new("/bin/sh")).is_err());
    }
}
