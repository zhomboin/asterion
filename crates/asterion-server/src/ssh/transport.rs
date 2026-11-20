use super::config::Target;
use asterion_protocol::{TaskProgress, TaskReply};
use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

// 请求和模块均走标准输入。远端命令行固定，SSH 私钥只由中心 OpenSSH 读取。
const REMOTE: &str = concat!(
    "python3 -I -B -u -c '",
    include_str!("../../../../deploy/ssh/bootstrap.py"),
    "'"
);
const MAX_REPLY: usize = 262144;

pub(super) async fn call(
    target: &Target,
    kind: &str,
    args: Value,
    seconds: u64,
    progress: Option<&watch::Sender<Option<TaskProgress>>>,
) -> TaskReply {
    let modules = [
        (
            "common",
            include_str!("../../../../deploy/managed/common.py"),
        ),
        ("host", include_str!("../../../../deploy/managed/host.py")),
        (
            "containers",
            include_str!("../../../../deploy/managed/containers.py"),
        ),
        ("monitor", include_str!("../../../../deploy/ssh/monitor.py")),
        (
            "docker_read",
            include_str!("../../../../deploy/ssh/docker_read.py"),
        ),
        (
            "accounts",
            include_str!("../../../../deploy/ssh/accounts.py"),
        ),
        ("actions", include_str!("../../../../deploy/ssh/actions.py")),
        (
            "dispatch",
            include_str!("../../../../deploy/ssh/dispatch.py"),
        ),
    ];
    let lightweight = matches!(
        kind,
        "sample" | "terminal_capability" | "process" | "processes" | "systemd" | "systemd_service"
    );
    let modules: Vec<_> = modules
        .into_iter()
        .filter(|(name, _)| !lightweight || matches!(*name, "common" | "monitor" | "dispatch"))
        .collect();
    let mut request = json!({"modules":modules, "kind":kind, "args":args,
        "policy":if lightweight {Value::Null} else {target.policy.clone()}, "seconds":seconds})
    .to_string()
    .into_bytes();
    if request.len() > 520000 {
        return TaskReply::failure("invalid_argument", "SSH 请求过大");
    }
    request.push(b'\n');
    let io = async {
        let mut child = target
            .command(false)
            .arg(REMOTE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
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
                return Ok(TaskReply::failure("too_large", "SSH 响应过大"));
            }
            let value: Value = serde_json::from_slice(&line)?;
            if let Some(data) = value.get("progress") {
                if let (Some(tx), Ok(value)) = (progress, serde_json::from_value(data.clone())) {
                    tx.send_replace(Some(value));
                }
            } else {
                reply = Some(serde_json::from_value(value)?);
            }
        }
        let status = child.wait().await?;
        drop(stdin);
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
