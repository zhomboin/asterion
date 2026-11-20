use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};

pub const DEFAULT_DATABASE_URL: &str = "sqlite://.local/asterion.db";
const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8080";

pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub web_dir: Option<PathBuf>,
    pub tls: Option<(PathBuf, PathBuf)>,
    pub admin_username: Option<String>,
    pub admin_password_file: Option<PathBuf>,
    pub sample_interval_secs: u64,
    pub retention_seconds: i64,
    pub trend_retention_seconds: i64,
    pub terminal_enabled: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let listen_addr: SocketAddr = env_value("ASTERION_LISTEN_ADDR")?
            .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_owned())
            .parse()
            .context("ASTERION_LISTEN_ADDR 必须是有效的监听地址")?;
        let database_url =
            env_value("ASTERION_DATABASE_URL")?.unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned());
        let web_dir = env_path("ASTERION_WEB_DIR");
        if let Some(directory) = &web_dir {
            ensure!(
                directory.join("index.html").is_file(),
                "ASTERION_WEB_DIR 必须指向包含 index.html 的前端构建目录"
            );
        }

        let local = match env_value("ASTERION_LOCAL")?.as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            _ => anyhow::bail!("ASTERION_LOCAL 必须为 0 或 1"),
        };
        ensure!(
            !local || listen_addr.ip().is_loopback(),
            "本地模式只能监听回环地址"
        );
        let tls = if local {
            None
        } else {
            Some((
                required_path("ASTERION_TLS_CERT_FILE")?,
                required_path("ASTERION_TLS_KEY_FILE")?,
            ))
        };
        let terminal_enabled = match env_value("ASTERION_TERMINAL_ENABLED")?.as_deref() {
            None => local,
            Some("0") => false,
            Some("1") => true,
            Some(_) => anyhow::bail!("ASTERION_TERMINAL_ENABLED 必须为 0 或 1"),
        };
        ensure!(
            !terminal_enabled || private_address(listen_addr.ip()),
            "启用终端时中心必须监听回环或私有网络地址"
        );

        Ok(Self {
            terminal_enabled,
            listen_addr,
            database_url,
            web_dir,
            tls,
            admin_username: env_value("ASTERION_ADMIN_USERNAME")?,
            admin_password_file: env_path("ASTERION_ADMIN_PASSWORD_FILE"),
            sample_interval_secs: bounded_env(
                "ASTERION_SAMPLE_INTERVAL_SECS",
                15,
                5,
                asterion_protocol::MAX_SAMPLE_INTERVAL_SECS,
            )?,
            retention_seconds: (bounded_env("ASTERION_METRIC_RETENTION_DAYS", 7, 1, 365)? * 86_400)
                as i64,
            trend_retention_seconds: (bounded_env("ASTERION_TREND_RETENTION_DAYS", 90, 30, 365)?
                * 86_400) as i64,
        })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn required_path(name: &str) -> Result<PathBuf> {
    env_path(name).with_context(|| format!("请配置 {name}"))
}

fn env_value(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("{name} 必须是有效的 UTF-8 字符串")),
    }
}

fn bounded_env(name: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    let Some(value) = env_value(name)? else {
        return Ok(default);
    };
    let value = value
        .parse::<u64>()
        .with_context(|| format!("{name} 必须是整数"))?;
    ensure!(
        (min..=max).contains(&value),
        "{name} 必须在 {min} 到 {max} 之间"
    );
    Ok(value)
}

fn private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_loopback()
                || ip.is_private()
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => ip.is_loopback() || ip.segments()[0] & 0xfe00 == 0xfc00,
    }
}
