use crate::{model::Node, ssh::Verifier, store::Store};
use anyhow::{Context, Result, bail, ensure};
use russh::{
    client,
    keys::{
        PrivateKeyWithHashAlg,
        agent::{AgentIdentity, client::AgentClient},
        load_secret_key,
    },
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;

#[derive(Clone)]
pub struct Config {
    pub node: Node,
    pub identities: Vec<PathBuf>,
    agent: Option<String>,
    identities_only: bool,
    pub trust: crate::host_trust::Trust,
    pub named: bool,
    pub jump: Option<String>,
    proxy: Option<String>,
    pub alias: String,
}
fn option<'a>(lines: &'a str, key: &str) -> Option<&'a str> {
    lines
        .lines()
        .filter_map(|l| l.split_once(' '))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim())
}
fn path(value: &str, node: &Node) -> Result<PathBuf> {
    let home = std::env::home_dir().context("找不到用户主目录")?;
    let expanded = value
        .trim_matches('"')
        .replace("%d", &home.to_string_lossy())
        .replace("%h", &node.host)
        .replace("%r", &node.user)
        .replace("%p", &node.port.to_string());
    Ok(expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
        .map_or_else(|| PathBuf::from(&expanded), |v| home.join(v)))
}
fn parse(output: &str) -> Result<Config> {
    let node = Node::parse(
        option(output, "hostname").context("SSH 配置缺少主机")?,
        option(output, "port").unwrap_or("22"),
        option(output, "user").context("SSH 配置缺少用户名")?,
    )?;
    ensure!(
        option(output, "proxyusefdpass") != Some("yes"),
        "标准输入输出代理无法接收文件描述符"
    );
    let identities = output
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(k, v)| *k == "identityfile" && *v != "none")
        .map(|(_, v)| path(v, &node))
        .collect::<Result<_>>()?;
    let mut files = Vec::new();
    let home = std::env::home_dir().context("找不到用户主目录")?;
    for key in ["userknownhostsfile", "globalknownhostsfile"] {
        if let Some(paths) = option(output, key) {
            let paths = paths
                .replace(home.to_string_lossy().as_ref(), "~")
                .replace(&home.to_string_lossy().replace(char::from(92), "/"), "~");
            for file in crate::local_program::arguments(&paths)? {
                if file != "none" {
                    files.push(path(
                        &file.replace(
                            "__PROGRAMDATA__",
                            &std::env::var("ProgramData").unwrap_or_default(),
                        ),
                        &node,
                    )?);
                }
            }
        }
    }
    Ok(Config {
        node,
        identities,
        identities_only: option(output, "identitiesonly") == Some("yes"),
        agent: option(output, "identityagent").map(str::to_owned),
        trust: crate::host_trust::Trust {
            files,
            host: option(output, "hostkeyalias").map(str::to_owned),
            required: option(output, "stricthostkeychecking") == Some("true")
                || option(output, "stricthostkeychecking") == Some("yes"),
        },
        proxy: option(output, "proxycommand")
            .filter(|s| *s != "none")
            .map(str::to_owned),
        alias: String::new(),
        named: false,
        jump: option(output, "proxyjump")
            .filter(|s| *s != "none")
            .map(str::to_owned),
    })
}
pub fn binary(name: &str) -> Command {
    #[cfg(windows)]
    let binary =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/OpenSSH")
            .join(format!("{name}.exe"));
    #[cfg(not(windows))]
    let binary = PathBuf::from("/usr/bin").join(name);
    let mut cmd = Command::new(binary);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd.kill_on_drop(true);
    cmd
}
pub fn ssh() -> Command {
    let mut cmd = binary("ssh");
    if let Some(config) = std::env::var_os("ASTERION_LOCAL_SSH_CONFIG") {
        cmd.arg("-F").arg(config);
    }
    cmd
}
fn profiles_file() -> Result<PathBuf> {
    match std::env::var_os("ASTERION_SSH_CONFIG_FILE") {
        Some(path) => Ok(path.into()),
        None => Ok(Store::directory()?.join("profiles.json")),
    }
}
pub(super) fn profiles(file: &Path) -> Result<Value> {
    use std::io::Read;
    let f = match std::fs::File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    f.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4 * 1024 * 1024, "命名连接文件过大");
    let v: Value = serde_json::from_slice(&bytes).context("命名连接文件格式错误")?;
    ensure!(v.is_object(), "命名连接须按名称登记");
    Ok(v)
}
pub fn names() -> Result<Vec<String>> {
    Ok(profiles(&profiles_file()?)?
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .filter(|n| !n.starts_with("ssh://"))
        .cloned()
        .collect())
}
pub(super) fn profile_node(p: &Value) -> Result<Node> {
    let port = p
        .get("port")
        .map(Value::to_string)
        .unwrap_or_else(|| "22".into());
    Node::parse(
        p["host"].as_str().context("命名连接缺少主机")?,
        &port,
        p["user"].as_str().context("命名连接缺少用户")?,
    )
}
pub async fn resolve(name: String) -> Result<Config> {
    ensure!(
        !name.is_empty()
            && name.len() <= 253
            && !name.starts_with('-')
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_:".contains(&c)),
        "请输入有效的连接名"
    );
    let file = profiles_file()?;
    if let Some(p) = profiles(&file)?.get(&name) {
        let node = profile_node(p)?;
        let dir = file.parent().context("无法解析配置目录")?;
        let known = dir.join(p["known_hosts_file"].as_str().unwrap_or("known_hosts"));
        ensure!(known.is_file(), "命名连接的 known_hosts 文件不存在");
        return Ok(Config {
            node,
            identities: vec![dir.join(p["identity_file"].as_str().unwrap_or("ssh.key"))],
            identities_only: true,
            agent: Some("none".into()),
            trust: crate::host_trust::Trust {
                files: vec![known],
                host: None,
                required: true,
            },
            proxy: None,
            alias: name.clone(),
            named: true,
            jump: None,
        });
    }
    let result = tokio::time::timeout(
        Duration::from_secs(6),
        ssh()
            .args(["-G", "-o", "PermitLocalCommand=no", "--", &name])
            .output(),
    )
    .await
    .context("解析 SSH 配置超时")??;
    ensure!(result.status.success(), "OpenSSH 无法解析本机配置");
    let mut parsed = parse(std::str::from_utf8(&result.stdout)?)?;
    parsed.alias = name;
    Ok(parsed)
}
impl Config {
    pub fn transport(&self) -> Result<Option<crate::jump::Stream>> {
        if let Some(cmd) = &self.proxy {
            return Ok(Some(crate::jump::start(crate::local_program::proxy(
                cmd,
                &self.node,
                &self.alias,
            )?)?));
        }
        self.jump
            .as_deref()
            .map(|jump| crate::jump::connect(jump, &self.node))
            .transpose()
    }
    pub async fn authenticate(&self, s: &mut client::Handle<Verifier>) -> Result<()> {
        let hash = s.best_supported_rsa_hash().await?.flatten();
        if self.agent.as_deref() != Some("none") {
            #[cfg(windows)]
            let agent = AgentClient::connect_named_pipe(
                self.agent
                    .clone()
                    .or_else(|| std::env::var("SSH_AUTH_SOCK").ok())
                    .unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".into()),
            )
            .await;
            #[cfg(not(windows))]
            let agent = match &self.agent {
                Some(path) => AgentClient::connect_uds(path).await,
                None => AgentClient::connect_env().await,
            };
            if let Ok(mut agent) = agent {
                for identity in agent.request_identities().await.unwrap_or_default() {
                    if let AgentIdentity::PublicKey { key, .. } = identity {
                        let allowed = !self.identities_only
                            || self.identities.iter().any(|p| {
                                russh::keys::load_public_key(format!("{}.pub", p.display()))
                                    .is_ok_and(|k| k.key_data() == key.key_data())
                            });
                        if allowed
                            && s.authenticate_publickey_with(&self.node.user, key, hash, &mut agent)
                                .await
                                .is_ok_and(|r| r.success())
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }
        for p in &self.identities {
            if let Ok(key) = load_secret_key(p, None)
                && s.authenticate_publickey(
                    &self.node.user,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await?
                .success()
            {
                return Ok(());
            }
        }
        bail!("本机 SSH 身份认证失败，请核对私钥路径或解锁 SSH Agent 中的密钥")
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn configuration_preserves_effective_destination_and_proxy() {
        let c=super::parse("hostname example.invalid\nuser operator\nport 2222\nidentityfile ~/.ssh/id_ed25519\nproxyjump gateway\nidentitiesonly yes").unwrap();
        assert_eq!(c.node.to_string(), "operator@example.invalid:2222");
        assert_eq!(c.jump.as_deref(), Some("gateway"));
        assert!(c.identities_only);
        assert!(
            super::parse("hostname example.invalid\nuser operator\nproxyusefdpass yes").is_err()
        );
    }
}
