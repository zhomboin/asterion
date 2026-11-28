use anyhow::{Result, bail, ensure};
use asterion_protocol::{TaskReply, collector};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl Node {
    pub fn from_address(address: &str) -> Result<Self> {
        let (user, destination) = address
            .strip_prefix("ssh://")
            .and_then(|s| s.rsplit_once('@'))
            .ok_or_else(|| anyhow::anyhow!("已保存的 SSH 地址无效"))?;
        let (host, port) = destination
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("已保存的 SSH 端口无效"))?;
        let host = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host);
        let node = Self::parse(host, port, user)?;
        ensure!(
            format!("ssh://{node}") == address,
            "已保存的 SSH 地址格式不一致"
        );
        Ok(node)
    }
    pub fn parse(host: &str, port: &str, user: &str) -> Result<Self> {
        let host = host.trim().to_ascii_lowercase();
        let user = user.trim();
        ensure!(
            !host.is_empty()
                && host.len() <= 253
                && !host.starts_with('-')
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b)),
            "请输入主机名或 IP"
        );
        ensure!(
            !user.is_empty()
                && user.len() <= 128
                && !user.chars().any(|c| c.is_control() || c.is_whitespace()),
            "请输入有效的 SSH 用户名"
        );
        let port = port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| anyhow::anyhow!("端口范围为 1–65535"))?;
        Ok(Self {
            host,
            port,
            user: user.to_owned(),
        })
    }
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.host.contains(':') {
            write!(f, "{}@[{}]:{}", self.user, self.host, self.port)
        } else {
            write!(f, "{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub at: i64,
    pub cpu: Option<f64>,
    pub memory: Option<f64>,
    pub disk: Option<f64>,
}

fn used(value: &Value) -> Option<f64> {
    let total = value["total_bytes"].as_u64()?;
    let available = value["available_bytes"].as_u64()?;
    (total > 0).then(|| 100.0 * total.saturating_sub(available) as f64 / total as f64)
}

pub fn decode_sample(bytes: &[u8]) -> Result<Sample> {
    let TaskReply::Success { value } = serde_json::from_slice(bytes)? else {
        bail!("节点采样失败");
    };
    from_value(&value["sample"])
}

pub fn from_value(s: &Value) -> Result<Sample> {
    ensure!(collector::valid_sample(s), "节点返回的采样格式无效");
    Ok(Sample {
        at: s["captured_at"].as_i64().unwrap(),
        cpu: s["cpu_percent"].as_f64(),
        memory: used(&s["memory"]),
        disk: s["filesystems"]
            .as_array()
            .and_then(|v| v.iter().find(|f| f["mountpoint"] == "/"))
            .and_then(used),
    })
}

pub const HISTORY: usize = 120;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nodes_are_normalized_and_ports_checked() {
        assert_eq!(
            Node::parse(" EXAMPLE.invalid ", "22", "operator")
                .unwrap()
                .host,
            "example.invalid"
        );
        for port in ["0", "65536", ""] {
            assert!(Node::parse("host", port, "u").is_err());
        }
        assert!(Node::parse("-oBad", "22", "u").is_err());
    }
    #[test]
    fn invalid_or_empty_samples_are_rejected() {
        assert!(decode_sample(br#"{"outcome":"success","value":{"sample":{}}}"#).is_err());
        assert_eq!(
            used(&serde_json::json!({"total_bytes":0,"available_bytes":0})),
            None
        );
    }
}
