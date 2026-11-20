use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

pub(super) type Profiles = BTreeMap<String, Target>;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    host: String,
    user: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "identity")]
    identity_file: PathBuf,
    #[serde(default = "hosts")]
    known_hosts_file: PathBuf,
    #[serde(default)]
    pub policy: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Target {
    Profile(Profile),
    Direct(super::direct::Direct, Value),
}

fn default_port() -> u16 {
    22
}
fn identity() -> PathBuf {
    "ssh.key".into()
}
fn hosts() -> PathBuf {
    "known_hosts".into()
}

pub(crate) fn valid_profile(name: &str) -> bool {
    name.len() <= 64
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub(super) async fn load(local: bool) -> Result<Profiles> {
    let Some(file) = std::env::var_os("ASTERION_SSH_CONFIG_FILE") else {
        return Ok(Profiles::new());
    };
    let bytes = match tokio::fs::read(&file).await {
        Err(e) if local && e.kind() == std::io::ErrorKind::NotFound => return Ok(Profiles::new()),
        result => result.context("无法读取 SSH 配置")?,
    };
    ensure!(bytes.len() <= 4 * 1024 * 1024, "SSH 配置过大");
    let entries: BTreeMap<String, Value> =
        serde_json::from_slice(&bytes).context("SSH 配置格式无效")?;
    let mut profiles = Profiles::new();
    let dir = Path::new(&file).parent().unwrap_or(Path::new("."));
    for (name, entry) in entries {
        if local && let Some(direct) = super::direct::Direct::parse(&name) {
            ensure!(
                entry.as_object().is_some_and(|obj| obj.len() == 1) && entry["policy"].is_object(),
                "直连地址仅接受 policy 配置"
            );
            profiles.insert(name, Target::Direct(direct, entry["policy"].clone()));
            continue;
        }
        let mut target: Profile = serde_json::from_value(entry).context("SSH 配置格式无效")?;
        ensure!(
            valid_profile(&name)
                && target.port > 0
                && valid_profile(&target.user)
                && !target.host.is_empty()
                && target.host.len() <= 253
                && !target.host.starts_with('-')
                && target
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'.' | b'_')),
            "SSH 名称、地址、账号或端口无效"
        );
        target.identity_file = dir.join(&target.identity_file);
        target.known_hosts_file = dir.join(&target.known_hosts_file);
        ensure!(
            target.known_hosts_file.to_str().is_some_and(|s| !s
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, '%' | '$' | '\'' | '"' | '\\'))),
            "known_hosts 路径包含特殊字符"
        );
        profiles.insert(name, Target::Profile(target));
    }
    Ok(profiles)
}

impl Target {
    pub(super) fn policy(&self) -> &Value {
        match self {
            Self::Profile(profile) => &profile.policy,
            Self::Direct(_, policy) => policy,
        }
    }
    pub(super) fn command(&self, pty: bool) -> Command {
        let mut command = Command::new("/usr/bin/ssh");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .args([
                "-oBatchMode=yes",
                "-oStrictHostKeyChecking=yes",
                "-oUpdateHostKeys=no",
                "-oConnectTimeout=5",
                "-oServerAliveInterval=5",
                "-oServerAliveCountMax=2",
                "-oClearAllForwardings=yes",
                "-oForwardAgent=no",
                "-oEscapeChar=none",
            ]);
        let host = match self {
            Self::Direct(direct, _) => direct.configure(&mut command),
            Self::Profile(profile) => {
                command
                    .args([
                        "-F",
                        "/dev/null",
                        "-oGlobalKnownHostsFile=/dev/null",
                        "-oIdentitiesOnly=yes",
                    ])
                    .arg(format!(
                        "-oUserKnownHostsFile={}",
                        profile.known_hosts_file.display()
                    ))
                    .arg("-i")
                    .arg(&profile.identity_file)
                    .arg("-p")
                    .arg(profile.port.to_string())
                    .arg("-l")
                    .arg(&profile.user);
                &profile.host
            }
        };
        command
            .arg(if pty { "-tt" } else { "-T" })
            .arg("--")
            .arg(host)
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }
}
