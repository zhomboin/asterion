use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

const PROGRAM: &str = include_str!("../../../../../deploy/ssh-enrollment/bootstrap.py");
pub(super) const UNIT: &str = include_str!("../../../../../deploy/asterion-agent.service");

pub(super) fn enabled() -> bool {
    [
        "ASTERION_SSH_DEPLOY_AGENT",
        "ASTERION_SSH_DEPLOY_CENTER_URL",
        "ASTERION_SSH_DEPLOY_NETWORKS",
    ]
    .iter()
    .all(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Plan {
    pub address: std::net::IpAddr,
    pub fingerprint: String,
    pub center: String,
    pub commands: Vec<String>,
    pub manifest: String,
}

pub(super) async fn call(
    request: Value,
    limit: Duration,
    mut shutdown: watch::Receiver<bool>,
    mut event: impl FnMut(Value) -> Result<(), String>,
) -> Result<(), String> {
    if *shutdown.borrow() {
        return Err("中心已停止".into());
    }
    let mut cmd = Command::new("/usr/bin/python3");
    cmd.env_clear();
    for key in [
        "ASTERION_SSH_DEPLOY_AGENT",
        "ASTERION_SSH_DEPLOY_CENTER_URL",
        "ASTERION_SSH_DEPLOY_NETWORKS",
        "ASTERION_SSH_DEPLOY_CA_FILE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    let mut child = cmd
        .args(["-I", "-B", "-u", "-c", PROGRAM])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "中心无法启动 Python SSH 部署器")?;
    let mut input = child.stdin.take().ok_or("部署器输入不可用")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("部署器输出不可用")?).lines();
    let action = async {
        input
            .write_all(&serde_json::to_vec(&request).map_err(|_| "部署请求无效")?)
            .await
            .map_err(|_| "部署器输入中断")?;
        drop(request);
        drop(input);
        let mut failure = None;
        while let Some(line) = lines.next_line().await.map_err(|_| "部署器输出中断")? {
            if line.len() > 16384 {
                return Err("部署器输出超限".to_owned());
            }
            let value: Value = serde_json::from_str(&line).map_err(|_| "部署器响应无效")?;
            if let Some(message) = value["error"].as_str() {
                failure = Some(message.to_owned());
            } else {
                event(value)?;
            }
        }
        let status = child.wait().await.map_err(|_| "部署器未正常结束")?;
        if let Some(message) = failure {
            Err(message)
        } else if !status.success() {
            Err("部署器异常结束".into())
        } else {
            Ok(())
        }
    };
    let answer = tokio::select! {
        result = tokio::time::timeout(limit, action) => result.unwrap_or_else(|_| Err("SSH 部署请求超时".into())),
        _ = shutdown.changed() => Err("中心已停止，SSH 会话已关闭".into()),
    };
    if answer.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    answer
}
