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
pub(super) struct Target {
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

pub(super) async fn load() -> Result<Profiles> {
    let Some(file) = std::env::var_os("ASTERION_SSH_CONFIG_FILE") else {
        return Ok(Profiles::new());
    };
    let bytes = tokio::fs::read(&file).await.context("无法读取 SSH 配置")?;
    ensure!(bytes.len() <= 4 * 1024 * 1024, "SSH 配置过大");
    let mut profiles: Profiles = serde_json::from_slice(&bytes).context("SSH 配置格式无效")?;
    let dir = Path::new(&file).parent().unwrap_or(Path::new("."));
    for (name, target) in &mut profiles {
        ensure!(
            valid_profile(name)
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
    }
    Ok(profiles)
}

impl Target {
    pub(super) fn command(&self, pty: bool) -> Command {
        let mut command = Command::new("/usr/bin/ssh");
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "xterm-256color")
            .args([
                "-F",
                "/dev/null",
                "-oBatchMode=yes",
                "-oStrictHostKeyChecking=yes",
                "-oGlobalKnownHostsFile=/dev/null",
                "-oUpdateHostKeys=no",
                "-oIdentitiesOnly=yes",
                "-oConnectTimeout=5",
                "-oServerAliveInterval=5",
                "-oServerAliveCountMax=2",
                "-oClearAllForwardings=yes",
                "-oForwardAgent=no",
                "-oEscapeChar=none",
            ])
            .arg(format!(
                "-oUserKnownHostsFile={}",
                self.known_hosts_file.display()
            ))
            .arg("-i")
            .arg(&self.identity_file)
            .arg("-p")
            .arg(self.port.to_string())
            .arg("-l")
            .arg(&self.user)
            .arg(if pty { "-tt" } else { "-T" })
            .arg("--")
            .arg(&self.host)
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }
}
