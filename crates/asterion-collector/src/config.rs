use anyhow::{Context, Result, ensure};
use std::{env, path::PathBuf};

pub struct Policy {
    pub interval: u64,
    pub retention: i64,
    pub max_bytes: i64,
}

impl Policy {
    pub fn load() -> Result<Self> {
        Ok(Self {
            interval: number("ASTERION_COLLECTOR_INTERVAL_SECS", 15, 5, 3600)? as u64,
            retention: number("ASTERION_COLLECTOR_RETENTION_DAYS", 7, 1, 365)? * 86400,
            max_bytes: number("ASTERION_COLLECTOR_MAX_MIB", 256, 1, 16384)? * 1024 * 1024,
        })
    }
}

fn number(key: &str, default: i64, min: i64, max: i64) -> Result<i64> {
    let raw = match env::var(key) {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return Ok(default),
        Err(err) => return Err(err.into()),
    };
    let n = raw.parse().with_context(|| format!("{key} 必须是整数"))?;
    ensure!((min..=max).contains(&n), "{key} 必须在 {min} 到 {max} 之间");
    Ok(n)
}

pub fn database() -> PathBuf {
    env::var_os("ASTERION_COLLECTOR_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/var/lib/asterion-collector/history.db".into())
}
