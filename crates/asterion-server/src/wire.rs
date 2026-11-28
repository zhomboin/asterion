use asterion_protocol::TaskReply;
use serde_json::{Value, json};
// 请求和模块均走标准输入。远端命令行固定，SSH 私钥只由中心 OpenSSH 读取。
const REMOTE: &str = concat!(
    "python3 -I -B -u -c '",
    include_str!("../../../deploy/ssh/bootstrap.py"),
    "'"
);
pub const MAX_REPLY: usize = 262144;

pub fn request(
    kind: &str,
    args: Value,
    seconds: u64,
    policy: &Value,
    direct: bool,
) -> Result<(String, Vec<u8>), TaskReply> {
    let modules = [
        ("common", include_str!("../../../deploy/managed/common.py")),
        ("host", include_str!("../../../deploy/managed/host.py")),
        (
            "containers",
            include_str!("../../../deploy/managed/containers.py"),
        ),
        ("monitor", include_str!("../../../deploy/ssh/monitor.py")),
        (
            "docker_read",
            include_str!("../../../deploy/ssh/docker_read.py"),
        ),
        ("accounts", include_str!("../../../deploy/ssh/accounts.py")),
        (
            "nginx_config",
            include_str!("../../../deploy/ssh/nginx_config.py"),
        ),
        (
            "text_files",
            include_str!("../../../deploy/ssh/text_files.py"),
        ),
        ("actions", include_str!("../../../deploy/ssh/actions.py")),
        ("dispatch", include_str!("../../../deploy/ssh/dispatch.py")),
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
        policy.clone()
    };
    if !lightweight && direct && policy.is_null() {
        policy["file_editor"] = Value::Bool(true);
    }
    let mut request = json!({"modules":modules, "kind":kind, "args":args,
        "policy":policy, "seconds":seconds})
    .to_string()
    .into_bytes();
    if request.len() > 520000 {
        return Err(TaskReply::failure("invalid_argument", "SSH 请求过大"));
    }
    request.push(b'\n');
    let local_command;
    let remote = if direct {
        let fallback = match kind {
            "sample" => include_str!("../../../deploy/ssh/sample-basic.sh"),
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
            "if test -x /usr/local/bin/asterion-collector; then exec /usr/local/bin/asterion-collector export; fi\nif command -v asterion-collector >/dev/null 2>&1; then exec asterion-collector export; fi\n{remote}"
        );
        collector_command.as_str()
    } else {
        remote
    };
    Ok((remote.to_owned(), request))
}
