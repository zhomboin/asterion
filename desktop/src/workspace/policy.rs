use crate::{model::Node, ssh::Session, ssh_config};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Policy {
    desktop_file: PathBuf,
    profiles_file: PathBuf,
    address: String,
    alias: Option<String>,
    named: Option<(String, Node)>,
}
impl Policy {
    pub fn new(directory: &Path, address: &str, session: &Session) -> Self {
        let config = session.config();
        Self {
            desktop_file: directory.join("policies.json"),
            profiles_file: std::env::var_os("ASTERION_SSH_CONFIG_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| directory.join("profiles.json")),
            address: address.into(),
            alias: config
                .filter(|c| !c.named)
                .map(|c| format!("ssh://{}", c.alias)),
            named: config
                .filter(|c| c.named)
                .map(|c| (c.alias.clone(), c.node.clone())),
        }
    }
    pub async fn read(self: &Arc<Self>) -> Result<Value> {
        let source = self.clone();
        tokio::task::spawn_blocking(move || source.load()).await?
    }
    fn load(&self) -> Result<Value> {
        let entries = ssh_config::profiles(&self.profiles_file)?;
        if let Some((name, expected)) = &self.named {
            let p = entries.get(name).context("命名连接已移除")?;
            ensure!(
                &ssh_config::profile_node(p)? == expected,
                "命名连接地址已改变，请重新核对目标"
            );
            return Ok(match p.get("policy") {
                Some(policy) if policy.is_object() => policy.clone(),
                None => json!({}),
                _ => anyhow::bail!("命名连接的策略格式错误"),
            });
        }
        let local = ssh_config::profiles(&self.desktop_file)?;
        let entry = local
            .get(&self.address)
            .or_else(|| entries.get(&self.address))
            .or_else(|| self.alias.as_ref().and_then(|a| entries.get(a)));
        match entry {
            None => Ok(Value::Null),
            Some(v) if v["policy"].is_object() => Ok(v["policy"].clone()),
            Some(_) => anyhow::bail!("所选地址的策略格式错误"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reloads_policy_and_denies_removed_or_redirected_named_connections() -> Result<()> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
        std::fs::create_dir_all(&base)?;
        let dir = tempfile::tempdir_in(base)?;
        let mut policy = Policy {
            desktop_file: dir.path().join("local"),
            profiles_file: dir.path().join("profiles"),
            address: "ssh://example".into(),
            alias: None,
            named: None,
        };
        let profile = json!({"ssh://example":{"policy":{"file_editor":false}},"name":{"host":"test.invalid","user":"operator"}});
        std::fs::write(&policy.profiles_file, serde_json::to_vec(&profile)?)?;
        assert_eq!(policy.load()?["file_editor"], false);
        std::fs::write(
            &policy.desktop_file,
            serde_json::to_vec(&json!({"ssh://example":{"policy":{"file_editor":true}}}))?,
        )?;
        assert_eq!(policy.load()?["file_editor"], true);
        policy.named = Some((
            "name".into(),
            Node::parse("test.invalid", "22", "operator")?,
        ));
        assert_eq!(policy.load()?, json!({}));
        let mut changed = profile;
        changed["name"]["host"] = json!("changed.invalid");
        std::fs::write(&policy.profiles_file, serde_json::to_vec(&changed)?)?;
        assert!(policy.load().is_err());
        std::fs::write(&policy.profiles_file, b"{}")?;
        assert!(policy.load().is_err());
        Ok(())
    }
}
