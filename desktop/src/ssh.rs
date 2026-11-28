use anyhow::{Context, Result, bail, ensure};
use russh::{
    Channel, ChannelMsg, client,
    keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, load_secret_key, ssh_key::HashAlg},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::oneshot, time::timeout};
use zeroize::Zeroizing;

use crate::model::{Node, Sample, decode_sample};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub enum Credentials {
    Password(Zeroizing<String>),
    Key(PathBuf, Zeroizing<String>),
}

pub struct Verifier {
    expected: Option<String>,
    observed: Option<oneshot::Sender<String>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        offered: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = offered else {
            return Ok(false);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let trusted = self.expected.as_deref() == Some(&fingerprint);
        if let Some(tx) = self.observed.take() {
            let _ = tx.send(fingerprint);
        }
        Ok(trusted)
    }
}

pub struct Connection {
    handle: client::Handle<Verifier>,
    node: Node,
    pin: String,
    auth: Credentials,
}
impl std::ops::Deref for Connection {
    type Target = client::Handle<Verifier>;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl Connection {
    pub async fn reconnect(&self) -> Result<Session> {
        let result = login(self.node.clone(), Some(self.pin.clone()), self.auth.clone()).await?;
        match result {
            Login::Connected(session) => Ok(session),
            Login::Verify(_) => bail!("部署后的主机指纹验证失败"),
        }
    }
}
pub type Session = Arc<Connection>;
pub type Shell = Channel<client::Msg>;

#[derive(Clone)]
pub enum Login {
    Verify(String),
    Connected(Session),
}

pub async fn login(node: Node, expected: Option<String>, auth: Credentials) -> Result<Login> {
    timeout(Duration::from_secs(25), async {
        let (tx, mut rx) = oneshot::channel();
        let verifier = Verifier {
            expected: expected.clone(),
            observed: Some(tx),
        };
        let config = client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 2,
            ..Default::default()
        };
        let mut session = match client::connect(
            Arc::new(config),
            (node.host.as_str(), node.port),
            verifier,
        )
        .await
        {
            Ok(session) => session,
            Err(err) => match rx.try_recv() {
                Ok(key) if expected.is_none() => return Ok(Login::Verify(key)),
                Ok(key) if expected.as_ref() != Some(&key) => {
                    bail!("服务器指纹与已信任记录不符，已阻止连接")
                }
                _ => return Err(err).context("SSH 握手失败，请检查地址、端口和网络"),
            },
        };
        let result = match auth.clone() {
            Credentials::Password(password) => {
                session
                    .authenticate_password(&node.user, password.as_str())
                    .await?
            }
            Credentials::Key(path, passphrase) => {
                let key = tokio::task::spawn_blocking(move || {
                    load_secret_key(
                        path,
                        (!passphrase.is_empty()).then_some(passphrase.as_str()),
                    )
                })
                .await?
                .context("无法打开 SSH 私钥，请检查路径、格式和私钥口令")?;
                let hash = session.best_supported_rsa_hash().await?.flatten();
                session
                    .authenticate_publickey(
                        &node.user,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
            }
        };
        ensure!(result.success(), "SSH 认证被拒绝，请检查账号和凭据");
        Ok(Login::Connected(Arc::new(Connection {
            handle: session,
            node,
            pin: expected.context("服务器尚未受信任")?,
            auth,
        })))
    })
    .await
    .context("SSH 连接超时")?
}

async fn command(session: &Session, command: &str, limit: usize) -> Result<Vec<u8>> {
    let mut channel = session.channel_open_session().await?;
    let result = timeout(Duration::from_secs(10), async {
        channel.exec(true, command).await?;
        let mut bytes = Vec::new();
        let mut status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    ensure!(bytes.len() + data.len() <= limit, "节点返回的数据过大");
                    bytes.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::Failure => bail!("服务器拒绝请求"),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        ensure!(status == Some(0), "远端命令未成功完成");
        Ok(bytes)
    })
    .await;
    let _ = channel.close().await;
    result.context("节点响应超时")?
}

pub async fn sample(session: Session) -> Result<Sample> {
    let bytes = command(
        &session,
        include_str!("../../deploy/ssh/sample-basic.sh"),
        64 * 1024,
    )
    .await?;
    decode_sample(&bytes)
}

pub async fn terminal(session: &Session, cols: u16, rows: u16) -> Result<Shell> {
    let uid = command(session, "id -u", 32).await?;
    let uid = std::str::from_utf8(&uid)?.trim().parse::<u32>()?;
    ensure!(uid != 0, "交互终端需要使用非 root SSH 账号");
    shell(session, cols, rows, None).await
}

pub async fn container_terminal(
    remote: &crate::workspace::remote::Remote,
    target: serde_json::Value,
    cols: u16,
    rows: u16,
) -> Result<Shell> {
    let reply = remote
        .query(
            "container_console_target",
            serde_json::json!({"container":target}),
            8,
            None,
        )
        .await;
    let asterion_protocol::TaskReply::Success { value } = reply else {
        bail!("容器终端未获授权或目标已变化")
    };
    let id = value
        .as_str()
        .filter(|id| asterion_protocol::is_lower_hex(id, 64))
        .context("容器身份无效")?;
    let command = format!(
        "exec /usr/bin/docker --host unix:///var/run/docker.sock exec -it --user 65534:65534 -- {id} /usr/bin/timeout -k 2 900 /bin/sh -i"
    );
    shell(&remote.current(), cols, rows, Some(command)).await
}

async fn shell(session: &Session, cols: u16, rows: u16, command: Option<String>) -> Result<Shell> {
    let mut channel = session.channel_open_session().await?;
    channel
        .request_pty(true, "xterm-256color", cols.into(), rows.into(), 0, 0, &[])
        .await?;
    acknowledge(&mut channel)
        .await
        .context("服务器无法分配 PTY")?;
    if let Some(cmd) = command {
        channel.exec(true, cmd).await?;
    } else {
        channel.request_shell(true).await?;
    }
    acknowledge(&mut channel)
        .await
        .context("服务器未打开登录 Shell")?;
    Ok(channel)
}

async fn acknowledge(channel: &mut Shell) -> Result<()> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::WindowAdjusted { .. } | ChannelMsg::XonXoff { .. }) => continue,
            _ => bail!("服务器未接受交互终端请求"),
        }
    }
}
