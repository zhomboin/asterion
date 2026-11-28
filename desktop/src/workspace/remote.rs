use crate::ssh::{Session, Shell};
use anyhow::{Context, Result, bail, ensure};
use asterion_protocol::{TaskProgress, TaskReply};
use asterion_server::{collector_deploy, wire};
use russh::ChannelMsg;
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{fs::File, io::AsyncReadExt, sync::watch};

#[derive(Clone)]
pub struct Remote {
    pub session: Session,
    policy: Arc<Value>,
    renewed: Arc<Renewed>,
}

struct Renewed {
    session: std::sync::Mutex<Option<Session>>,
    runtime: tokio::runtime::Handle,
}
impl Renewed {
    fn new() -> Self {
        Self {
            session: std::sync::Mutex::new(None),
            runtime: tokio::runtime::Handle::current(),
        }
    }
    fn replace(&self, next: Option<Session>) {
        let previous = std::mem::replace(
            &mut *self.session.lock().unwrap_or_else(|e| e.into_inner()),
            next,
        );
        if let Some(old) = previous {
            self.runtime.spawn(async move {
                let _ = old
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
            });
        }
    }
}
impl Drop for Renewed {
    fn drop(&mut self) {
        self.replace(None);
    }
}

// 在超时、取消和节点切换时也关闭请求的 SSH 通道。
struct Guard(Option<Shell>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(channel) = self.0.take() {
            tokio::spawn(async move {
                let _ = channel.close().await;
            });
        }
    }
}

impl Remote {
    pub fn current(&self) -> Session {
        self.renewed
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| self.session.clone())
    }
    pub async fn new(session: Session, directory: &Path, address: &str) -> Result<Self> {
        let policy = match tokio::fs::File::open(directory.join("policies.json")).await {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                ensure!(bytes.len() <= 4 * 1024 * 1024, "节点策略文件过大");
                let entries: Value =
                    serde_json::from_slice(&bytes).context("节点策略文件格式错误")?;
                ensure!(entries.is_object(), "节点策略必须按 SSH 地址登记");
                if let Some(entry) = entries.get(address) {
                    ensure!(entry["policy"].is_object(), "所选节点的策略格式错误");
                    entry["policy"].clone()
                } else {
                    Value::Null
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Null,
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            session,
            policy: Arc::new(policy),
            renewed: Arc::new(Renewed::new()),
        })
    }

    pub async fn query(
        &self,
        kind: &str,
        args: Value,
        seconds: u64,
        progress: Option<&watch::Sender<Option<TaskProgress>>>,
    ) -> TaskReply {
        let (command, bytes) = match wire::request(kind, args, seconds, &self.policy, true) {
            Ok(parts) => parts,
            Err(reply) => return reply,
        };
        let read = async {
            let mut guard = Guard(Some(self.current().channel_open_session().await?));
            let channel = guard.0.as_mut().unwrap();
            channel.exec(true, command).await?;
            channel.data_bytes(bytes).await?;
            let mut line = Vec::new();
            let mut result = None;
            let mut code = None;
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => {
                        for chunk in data.split_inclusive(|b| *b == b'\n') {
                            ensure!(line.len() + chunk.len() <= wire::MAX_REPLY, "SSH 响应过大");
                            line.extend_from_slice(chunk);
                            if line.last() == Some(&b'\n') {
                                let row: Value = serde_json::from_slice(&line)?;
                                if let Some(item) = row.get("progress") {
                                    if let Some(tx) = progress {
                                        tx.send_replace(Some(serde_json::from_value(
                                            item.clone(),
                                        )?));
                                    }
                                } else {
                                    ensure!(result.is_none(), "重复的 SSH 响应");
                                    result = Some(serde_json::from_value::<TaskReply>(row)?);
                                }
                                line.clear();
                            }
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    ChannelMsg::Failure => bail!("远端拒绝执行请求"),
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            ensure!(line.is_empty() && code == Some(0), "远端请求中断");
            result.context("远端未返回结果")
        };
        match tokio::time::timeout(Duration::from_secs(seconds), read).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(e)) => TaskReply::failure("offline", &e.to_string()),
            Err(_) => TaskReply::failure("timeout", "SSH 请求超时"),
        }
    }
}

impl collector_deploy::Target for Remote {
    async fn exchange(
        &self,
        command: &str,
        upload: Option<(File, &str, &str)>,
    ) -> Result<(i32, String)> {
        let mut guard = Guard(Some(self.current().channel_open_session().await?));
        let channel = guard.0.as_mut().unwrap();
        channel.exec(true, command).await?;
        let mut bytes = Vec::new();
        let mut code = None;
        let mut upload = upload;
        if upload.is_none() {
            channel.eof().await?;
        }
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    ensure!(bytes.len() + data.len() <= 4096, "部署响应过大");
                    bytes.extend_from_slice(&data);
                    if upload.is_some() {
                        ensure!(bytes.len() <= 80, "部署握手异常");
                        if bytes.ends_with(b"\n") {
                            let (mut file, secret, revision) = upload.take().unwrap();
                            ensure!(
                                bytes == format!("READY {revision}\n").as_bytes(),
                                "部署目标未确认"
                            );
                            bytes.clear();
                            channel.data_bytes(format!("{secret}\n")).await?;
                            let mut buf = vec![0; 65536];
                            loop {
                                let n = file.read(&mut buf).await?;
                                if n == 0 {
                                    break;
                                }
                                channel.data_bytes(buf[..n].to_vec()).await?;
                            }
                            channel.eof().await?;
                        }
                    }
                }
                ChannelMsg::ExitStatus { exit_status } => code = Some(i32::try_from(exit_status)?),
                ChannelMsg::Failure => bail!("远端拒绝部署请求"),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        let code = code.context("部署连接已中断")?;
        ensure!(code != 0 || upload.is_none(), "缺少部署握手");
        Ok((code, String::from_utf8(bytes)?))
    }
    async fn verify_sample(&self) -> TaskReply {
        let session = match self.session.reconnect().await {
            Ok(session) => session,
            Err(_) => return TaskReply::failure("offline", "无法用已核对指纹的新连接读取采样"),
        };
        let fresh = Self {
            session,
            policy: self.policy.clone(),
            renewed: Arc::new(Renewed::new()),
        };
        let reply = fresh
            .query("sample", json!({"stream":"","seq":0}), 10, None)
            .await;
        if matches!(&reply,TaskReply::Success{value} if value["collector"].is_object()) {
            self.renewed.replace(Some(fresh.session.clone()));
        } else {
            let _ = fresh
                .session
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
        }
        reply
    }
}
