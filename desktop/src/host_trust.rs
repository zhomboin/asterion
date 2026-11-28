use crate::model::Node;
use anyhow::{Context, Result, bail, ensure};
use russh::keys::ssh_key::PublicKey;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Trust {
    pub files: Vec<PathBuf>,
    pub host: Option<String>,
    pub required: bool,
}
impl Trust {
    pub async fn check(&self, n: &Node, key: &PublicKey) -> Result<()> {
        let target = self.host.clone().unwrap_or_else(|| {
            if n.port == 22 {
                n.host.clone()
            } else {
                format!("[{}]:{}", n.host, n.port)
            }
        });
        let mut observed = false;
        let mut matched = false;
        for file in &self.files {
            if !file.try_exists()? {
                continue;
            }
            let mut cmd = crate::ssh_config::binary("ssh-keygen");
            let result = cmd
                .arg("-F")
                .arg(&target)
                .arg("-f")
                .arg(file)
                .output()
                .await
                .context("无法读取 OpenSSH 主机指纹")?;
            if result.status.code() == Some(1) && result.stderr.is_empty() {
                continue;
            }
            ensure!(result.status.success(), "OpenSSH 无法读取已知主机记录");
            for line in std::str::from_utf8(&result.stdout)?
                .lines()
                .filter(|s| !s.starts_with('#') && !s.is_empty())
            {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                let marked = fields.first().is_some_and(|s| s.starts_with('@'));
                let index = if marked { 2 } else { 1 };
                let algorithm = fields.get(index).context("已知主机记录格式无效")?;
                let data = fields.get(index + 1).context("已知主机公钥无效")?;
                let known = PublicKey::from_openssh(&format!("{algorithm} {data}"))
                    .context("无法核对已知主机公钥")?;
                let same = known.key_data() == key.key_data();
                if marked && fields[0] == "@revoked" && same {
                    bail!("服务器的 SSH 公钥已被撤销");
                }
                if !marked {
                    observed = true;
                    matched |= same;
                } else if fields[0] == "@cert-authority" {
                    observed = true;
                }
            }
        }
        ensure!(
            matched || (!observed && !self.required),
            "服务器公钥与 OpenSSH 信任记录不符"
        );
        Ok(())
    }
}
