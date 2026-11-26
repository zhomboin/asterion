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
        (
            "nginx_config",
            include_str!("../../../../deploy/ssh/nginx_config.py"),
        ),
        (
            "text_files",
            include_str!("../../../../deploy/ssh/text_files.py"),
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
    let mut policy = if lightweight {
        Value::Null
    } else {
        target.policy().clone()
    };
    if !lightweight && matches!(target, Target::Direct(..)) && policy.is_null() {
        policy["file_editor"] = Value::Bool(true);
    }
    let mut request = json!({"modules":modules, "kind":kind, "args":args,
        "policy":policy, "seconds":seconds})
    .to_string()
    .into_bytes();
    if request.len() > 520000 {
        return TaskReply::failure("invalid_argument", "SSH 请求过大");
    }
    request.push(b'\n');
    let local_command;
    let remote = if matches!(target, Target::Direct(..)) {
        let fallback = match kind {
            "sample" => include_str!("../../../../deploy/ssh/sample-basic.sh"),
            "terminal_capability" => {
                r#"enabled=false; if test "$(id -u)" != 0; then enabled=true; fi; printf '{"outcome":"success","value":{"enabled":%s}}\n' "$enabled""#
            }
            _ => {
                r#"printf '%s\n' '{"outcome":"failure","code":"unavailable","message":"此面板需要目标服务器提供 Python 3.9 或以上版本"}'"#
            }
        };
        local_command = format!(
            "if command -v python3 >/dev/null 2>&1; then {REMOTE}; status=$?; test \"$status\" = 79 || exit \"$status\"; fi\nIFS= read -r request || exit 1\n{fallback}"
        );
        local_command.as_str()
    } else {
        REMOTE
    };
    let collector_command;
    let remote = if kind == "sample" {
        collector_command = format!(
            "if command -v asterion-collector >/dev/null 2>&1; then exec asterion-collector export; fi\n{remote}"
        );
        collector_command.as_str()
    } else {
        remote
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
