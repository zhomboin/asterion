use std::{process::Stdio, time::Duration};

use anyhow::{Result, ensure};
use axum::{Extension, Json, http::StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::Mutex;

use super::{
    auth,
    config::{self, Target},
    direct::Direct,
};
use crate::{
    auth::{AdminSession, Csrf},
    local::LocalPanel,
};

const PROBE: &str = "ASTERION_HOST_KEY_PROBE";
const ACCEPT: &str = "ASTERION_HOST_KEY_ACCEPT";
static CONFIRM: Mutex<()> = Mutex::const_new(());
type Error = (StatusCode, Json<Value>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Check {
    ssh_profile: String,
    fingerprint: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum HostKey {
    Trusted,
    Unknown {
        algorithm: String,
        fingerprint: String,
    },
}

fn error(status: StatusCode, message: &str) -> Error {
    (status, Json(json!({ "message": message })))
}

fn valid_fingerprint(s: &str) -> bool {
    s.strip_prefix("SHA256:").is_some_and(|hash| {
        hash.len() == 43
            && STANDARD_NO_PAD
                .decode(hash)
                .is_ok_and(|bytes| bytes.len() == 32)
    })
}

pub fn is_confirmation_helper() -> bool {
    std::env::var_os(PROBE).is_some()
}

pub fn answer_prompt() -> Result<()> {
    auth::verify_askpass_owner()?;
    let prompt = std::env::args_os().nth(1).unwrap_or_default();
    let prompt = prompt.to_string_lossy();
    ensure!(
        prompt.starts_with("The authenticity of host '")
            && prompt.contains("Are you sure you want to continue connecting"),
        "不是首次主机确认提示"
    );
    let (algorithm, fingerprint) = prompt
        .lines()
        .find_map(|line| line.strip_suffix('.')?.split_once(" key fingerprint is "))
        .filter(|(algorithm, fp)| {
            !algorithm.is_empty()
                && algorithm.len() <= 64
                && algorithm
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && valid_fingerprint(fp)
        })
        .ok_or_else(|| anyhow::anyhow!("无法读取 SSH 主机指纹"))?;
    eprintln!("{} {algorithm} {fingerprint}", std::env::var(PROBE)?);
    let accept = std::env::var(ACCEPT).ok();
    println!(
        "{}",
        if accept.as_deref() == Some(fingerprint) {
            fingerprint
        } else {
            "no"
        }
    );
    Ok(())
}

pub(crate) async fn check(
    _: AdminSession,
    _: Csrf,
    _: Extension<LocalPanel>,
    Json(input): Json<Check>,
) -> Result<Json<HostKey>, Error> {
    let invalid = || error(StatusCode::BAD_REQUEST, "无效的 SSH 目标或指纹");
    let target = if let Some(direct) = Direct::parse(&input.ssh_profile) {
        Target::Direct(direct, Value::Null)
    } else if config::valid_profile(&input.ssh_profile) {
        config::load(true)
            .await
            .map_err(|_| invalid())?
            .remove(&input.ssh_profile)
            .ok_or_else(invalid)?
    } else {
        return Err(invalid());
    };
    if let Some(fingerprint) = input.fingerprint.as_deref() {
        if !valid_fingerprint(fingerprint) {
            return Err(invalid());
        }
        // 避免两个并发确认分别向同一份主机记录写入不同的首次密钥。
        let _guard = CONFIRM
            .try_lock()
            .map_err(|_| error(StatusCode::CONFLICT, "另一项主机确认正在处理，请重试"))?;
        if !matches!(
            probe(&target, Some(fingerprint), true).await?,
            HostKey::Trusted
        ) {
            return Err(error(
                StatusCode::CONFLICT,
                "服务器指纹在确认期间发生变化，请重新核对",
            ));
        }
        if !matches!(
            probe(&target, Some(fingerprint), false).await?,
            HostKey::Trusted
        ) {
            return Err(error(
                StatusCode::CONFLICT,
                "未能保存主机信任记录，请检查本机 known_hosts 的写入权限",
            ));
        }
        return Ok(Json(HostKey::Trusted));
    }
    probe(&target, None, false).await.map(Json)
}

async fn probe(
    target: &Target,
    expected: Option<&str>,
    allow_store: bool,
) -> Result<HostKey, Error> {
    let failed = || {
        error(
            StatusCode::BAD_GATEWAY,
            "无法验证 SSH 主机，请检查主机地址、端口和网络连接",
        )
    };
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).map_err(|_| failed())?;
    let marker = format!("asterion-host-key-{}", STANDARD_NO_PAD.encode(nonce));
    let mut cmd = Target::client();
    // 主客户端的诊断单独走 stdout，跳板进程的诊断不能被当作目标主机的握手结果。
    cmd.args([
        "-N",
        "-v",
        "-E",
        "/dev/stdout",
        "-oBatchMode=no",
        "-oStrictHostKeyChecking=ask",
        "-oFingerprintHash=sha256",
        "-oVisualHostKey=no",
        "-oPreferredAuthentications=none",
        "-oPasswordAuthentication=no",
        "-oKbdInteractiveAuthentication=no",
        "-oPubkeyAuthentication=no",
        "-oGSSAPIAuthentication=no",
        "-oHostbasedAuthentication=no",
    ])
    .env("LC_ALL", "C")
    .env("SSH_ASKPASS_REQUIRE", "force")
    .env(
        "SSH_ASKPASS",
        std::env::current_exe().map_err(|_| failed())?,
    )
    .env(auth::ASKPASS_OWNER, std::process::id().to_string())
    .env(PROBE, &marker);
    if allow_store && let Some(fp) = expected {
        cmd.env(ACCEPT, fp);
    }
    let mut child = target
        .destination_command(cmd, false)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| failed())?;
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap().take(64 * 1024);
    let mut messages = Vec::new();
    let observe = async {
        let mut lines = BufReader::new(stdout.take(128 * 1024)).lines();
        let mut trusted = false;
        let mut changed = false;
        let mut presented = None;
        while let Some(line) = lines.next_line().await? {
            if let Some(key) = line
                .strip_prefix("debug1: Server host key: ")
                .or_else(|| line.strip_prefix("debug1: Server host certificate: "))
            {
                presented = key
                    .split_whitespace()
                    .nth(1)
                    .map(|s| s.trim_end_matches(',').to_owned());
            }
            if line.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
                || line.contains("REVOKED HOST KEY")
            {
                changed = true;
            }
            if line.trim_end() == "debug1: SSH2_MSG_NEWKEYS received" {
                trusted = true;
                break;
            }
        }
        let _ = child.start_kill();
        Ok::<_, std::io::Error>((trusted, changed, presented))
    };
    let result = tokio::time::timeout(Duration::from_secs(7), async {
        tokio::try_join!(observe, stderr.read_to_end(&mut messages))
    })
    .await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    let ((trusted, changed, presented), _) = result.map_err(|_| failed())?.map_err(|_| failed())?;
    if changed {
        return Err(error(
            StatusCode::CONFLICT,
            "服务器主机密钥已变更或已撤销，已阻止连接。请联系管理员核实。",
        ));
    }
    if trusted {
        if expected.is_some() && presented.as_deref() != expected {
            return Err(error(
                StatusCode::CONFLICT,
                "本次握手指纹与确认的指纹不一致，已阻止连接",
            ));
        }
        return Ok(HostKey::Trusted);
    }
    for line in String::from_utf8_lossy(&messages).lines() {
        if let Some((algorithm, fingerprint)) = line
            .strip_prefix(&marker)
            .and_then(|s| s.strip_prefix(' '))
            .and_then(|s| s.split_once(' '))
            && valid_fingerprint(fingerprint)
        {
            return Ok(HostKey::Unknown {
                algorithm: algorithm.to_owned(),
                fingerprint: fingerprint.to_owned(),
            });
        }
    }
    Err(failed())
}
