use anyhow::{Context, Result, ensure};
use asterion_protocol::{
    TaskProgress, TaskReply, collector::Settings, is_token, operation::OperationAction,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::watch,
};

pub trait Target: Sync {
    fn exchange(
        &self,
        command: &str,
        upload: Option<(File, &str, &str)>,
    ) -> impl Future<Output = Result<(i32, String)>> + Send;
    fn verify_sample(&self) -> impl Future<Output = TaskReply> + Send;
}

const PROBE: &str = concat!(
    include_str!("../../../deploy/collector/identity.sh"),
    include_str!("../../../deploy/collector/probe.sh")
);
const RECEIVE: &str = concat!(
    include_str!("../../../deploy/collector/identity.sh"),
    include_str!("../../../deploy/collector/receive.sh")
);
const MAX_BINARY: u64 = 128 * 1024 * 1024;

fn shell_arg(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

struct Probe {
    arch: String,
    state: String,
    revision: String,
    root: bool,
    enabled: bool,
    settings: Option<Settings>,
    binary: Option<PathBuf>,
}

impl Probe {
    async fn read(target: &impl Target) -> Result<Self> {
        let command = format!("/bin/sh -c {}", shell_arg(PROBE));
        let (code, output) = target.exchange(&command, None).await?;
        ensure!(code == 0, "SSH 检查失败，请核对连接和节点读取权限");
        let lines: Vec<_> = output.lines().collect();
        ensure!(
            lines.len() == 8
                && matches!(lines[1], "unsupported" | "missing" | "active" | "inactive")
                && (is_token(lines[2]) || lines[1] == "unsupported")
                && lines[0].len() <= 64,
            "节点返回的部署状态无效"
        );
        Ok(Self {
            arch: lines[0].into(),
            state: lines[1].into(),
            revision: lines[2].into(),
            root: lines[3] == "0",
            enabled: lines[4] == "yes",
            settings: (|| {
                Some(Settings {
                    interval_secs: lines[5].parse().ok()?,
                    retention_days: lines[6].parse().ok()?,
                    max_mib: lines[7].parse().ok()?,
                })
            })()
            .filter(Settings::valid),
            binary: artifact(lines[0]).await,
        })
    }
}

async fn artifact(arch: &str) -> Option<PathBuf> {
    let arch = match arch {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let path = if cfg!(target_os = "linux") && arch == std::env::consts::ARCH {
        directory.join("asterion-collector")
    } else {
        directory
            .join("collectors")
            .join(arch)
            .join("asterion-collector")
    };
    let meta = tokio::fs::metadata(&path).await.ok()?;
    (meta.is_file() && (1..=MAX_BINARY).contains(&meta.len())).then_some(path)
}

pub async fn status(target: &impl Target) -> TaskReply {
    match Probe::read(target).await {
        Ok(p) => TaskReply::Success {
            value: json!({"arch":p.arch,"state":p.state,"revision":p.revision,
            "root":p.root,"enabled":p.enabled,"settings":p.settings,"artifact_available":p.binary.is_some()}),
        },
        Err(_) => TaskReply::failure("offline", "SSH 部署检查失败"),
    }
}

pub async fn deploy(
    target: &impl Target,
    args: Value,
    progress: &watch::Sender<Option<TaskProgress>>,
) -> TaskReply {
    let started = Instant::now();
    let result = install(target, args, progress).await;
    let (exit_code, output) = match result {
        Ok(()) => (
            0,
            "采集器已设为开机启动，已通过新的 SSH 会话读取本次启动后的采样。".to_owned(),
        ),
        Err(error) => (1, error.to_string()),
    };
    TaskReply::Success {
        value: json!({"exit_code":exit_code,"output":output,
        "duration_ms":started.elapsed().as_millis() as u64,"truncated":false}),
    }
}

async fn install(
    target: &impl Target,
    mut args: Value,
    progress: &watch::Sender<Option<TaskProgress>>,
) -> Result<()> {
    let action: OperationAction =
        serde_json::from_value(args["action"].take()).context("部署参数无效")?;
    ensure!(action.valid(), "部署参数无效");
    let OperationAction::CollectorDeploy { revision, settings } = action else {
        anyhow::bail!("部署操作无效")
    };
    let probe = Probe::read(target).await?;
    ensure!(
        probe.state != "unsupported",
        "节点需要提供 Linux、systemd 和 systemd-sysusers"
    );
    ensure!(
        probe.revision == revision,
        "节点或 SSH 账号已变化，请重新检查并确认部署"
    );
    let path = probe
        .binary
        .context("面板缺少匹配节点架构的采集器，请检查运行包")?;
    let mut file = File::open(path).await.context("无法打开面板随附的采集器")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65536];
    let mut bytes = 0;
    loop {
        let n = file.read(&mut buf).await.context("无法读取采集器")?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        ensure!(bytes <= MAX_BINARY, "采集器文件超过大小限制");
        hasher.update(&buf[..n]);
    }
    let digest = format!("{:x}", hasher.finalize());
    file.rewind().await.context("无法读取采集器")?;
    let root_script = include_str!("../../../deploy/collector/install.sh")
        .replace(
            "@UNIT@",
            include_str!("../../../deploy/collector/asterion-collector.service").trim_end(),
        )
        .replace(
            "@ACCOUNT@",
            include_str!("../../../deploy/collector/asterion-collector.sysusers").trim_end(),
        )
        .replace(
            "@DEFAULTS@",
            include_str!("../../../deploy/collector/asterion-collector.conf").trim_end(),
        );
    let limits = settings
        .map(|s| format!("{} {} {}", s.interval_secs, s.retention_days, s.max_mib))
        .unwrap_or_else(|| "'' '' ''".into());
    let cmd = format!(
        "/bin/sh -c {} receive {} {} {} {}",
        shell_arg(RECEIVE),
        revision,
        digest,
        shell_arg(&root_script),
        limits
    );
    progress.send_replace(Some(TaskProgress {
        output: "正在通过 SSH 传输采集器并配置 systemd 服务。".into(),
        truncated: false,
    }));
    let secret = if probe.root {
        ""
    } else {
        args["sudo_password"].as_str().unwrap_or_default()
    };
    let (code, out) = target
        .exchange(&cmd, Some((file, secret, &revision)))
        .await?;
    let failure = match code {
        0 => None,
        1 if out.lines().any(|line| line == "INSTALLING") => {
            Some("采集器安装失败，请检查远端磁盘和系统工具")
        }
        1 | 43 => Some("远端权限不足或 sudo 密码无效，请核对 SSH 账号的 sudo 权限并重试"),
        41 => Some("目标节点已变化，请重新检查部署状态"),
        42 => Some("传输校验失败，请重试"),
        44 => Some("此服务器已有部署任务正在运行"),
        46 => Some("采集器无法在此服务器运行，请检查架构和运行库"),
        47 => Some("部署路径包含符号链接或非常规文件，请核对服务器配置"),
        48 => Some("systemd 安装或启动失败，请查看服务器服务日志"),
        49 => Some("无法授予 SSH 账号读取采样的权限"),
        50 => Some("服务未生成可读取的新采样，已尝试恢复部署前的文件和服务状态"),
        255 => Some("SSH 连接中断，请重新检查节点状态"),
        _ => Some("采集器部署失败，请检查远端磁盘和系统工具"),
    };
    if let Some(message) = failure {
        anyhow::bail!(message)
    }
    let since = out
        .lines()
        .last()
        .and_then(|s| s.parse::<i64>().ok())
        .context("部署结果无法核对，请刷新节点状态")?;
    progress.send_replace(Some(TaskProgress {
        output: "服务已启动，正在用新的 SSH 会话验证采样读取权限。".into(),
        truncated: false,
    }));
    for _ in 0..5 {
        let reply = target.verify_sample().await;
        if let TaskReply::Success { value } = reply {
            let batch = &value["collector"];
            if batch["newest_at"].as_i64().is_some_and(|at| at >= since)
                && batch["records"]
                    .as_array()
                    .and_then(|rows| rows.first())
                    .is_some_and(|r| crate::monitor::valid_sample(&r["sample"]))
            {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!(
        "服务已安装，尚未从新 SSH 会话读到本次启动后的采样。请检查服务日志与采集器数据目录权限。"
    )
}
