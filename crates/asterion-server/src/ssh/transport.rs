use super::config::Target;
use asterion_protocol::{TaskProgress, TaskReply};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

use crate::wire::MAX_REPLY;

pub(super) async fn call(
    target: &Target,
    kind: &str,
    args: Value,
    seconds: u64,
    progress: Option<&watch::Sender<Option<TaskProgress>>>,
) -> TaskReply {
    let (remote, request) = match crate::wire::request(
        kind,
        args,
        seconds,
        target.policy(),
        matches!(target, Target::Direct(..)),
    ) {
        Ok(parts) => parts,
        Err(reply) => return reply,
    };
    let io = async {
        let password = target.password_revision().is_some();
        let mut child = target
            .command(false)
            .arg(remote)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if password {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .spawn()?;
        let stderr = child.stderr.take();
        let diagnostics = async move {
            let mut output = Vec::new();
            if let Some(stderr) = stderr {
                stderr.take(8192).read_to_end(&mut output).await?;
            }
            Ok::<_, std::io::Error>(output)
        };
        let exchange = async {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(&request).await?;
            let mut reader = BufReader::new(child.stdout.take().unwrap());
            let mut reply = None;
            loop {
                let mut line = Vec::new();
                let n = (&mut reader)
                    .take(MAX_REPLY as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await?;
                if n == 0 {
                    break;
                }
                if n > MAX_REPLY {
                    child.start_kill()?;
                    return Ok(Some(TaskReply::failure("too_large", "SSH 响应过大")));
                }
                let value: Value = serde_json::from_slice(&line)?;
                if let Some(data) = value.get("progress") {
                    if let (Some(tx), Ok(value)) = (progress, serde_json::from_value(data.clone()))
                    {
                        tx.send_replace(Some(value));
                    }
                } else {
                    reply = Some(serde_json::from_value(value)?);
                }
            }
            drop(stdin);
            Ok::<_, std::io::Error>(reply)
        };
        let (reply, diagnostics) = tokio::join!(exchange, diagnostics);
        let status = child.wait().await?;
        if status.code() == Some(255)
            && password
            && String::from_utf8_lossy(&diagnostics?)
                .lines()
                .any(|line| line.contains("Permission denied ("))
        {
            return Ok(TaskReply::failure(
                "password_denied",
                "服务器拒绝了 SSH 密码认证",
            ));
        }
        let reply = reply?;
        if matches!(&reply, Some(TaskReply::Failure { code, .. }) if code == "too_large") {
            return Ok(reply.unwrap());
        }
        Ok::<_, std::io::Error>(if status.success() {
            reply.unwrap_or_else(|| TaskReply::failure("invalid_response", "SSH 响应缺失"))
        } else if status.code() == Some(255) {
            TaskReply::failure("offline", "SSH 连接失败，请检查地址、主机密钥和登录凭据")
        } else {
            TaskReply::failure("unavailable", "SSH 远端需要 Python 3")
        })
    };
    match tokio::time::timeout(Duration::from_secs(seconds), io).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) => TaskReply::failure("offline", "SSH 会话中断或响应无效"),
        Err(_) => TaskReply::failure("timeout", "SSH 请求超时"),
    }
}
