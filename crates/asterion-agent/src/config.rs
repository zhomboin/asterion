use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use asterion_protocol::AGENT_CONNECT_PATH;
use url::Url;

pub struct Config {
    pub endpoint: Url,
    pub state_file: PathBuf,
    pub enrollment_file: Option<PathBuf>,
    pub ca_file: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let server_url = env::var("ASTERION_SERVER_URL").context("需要配置 ASTERION_SERVER_URL")?;
        let state_file = env::var_os("ASTERION_AGENT_STATE_FILE")
            .filter(|path| !path.is_empty())
            .context("需要配置 ASTERION_AGENT_STATE_FILE")?;

        Ok(Self {
            endpoint: agent_endpoint(&server_url)?,
            state_file: state_file.into(),
            enrollment_file: optional_path("ASTERION_AGENT_ENROLLMENT_FILE"),
            ca_file: optional_path("ASTERION_AGENT_CA_FILE"),
        })
    }
}

fn optional_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn agent_endpoint(value: &str) -> Result<Url> {
    ensure!(
        value.starts_with("https://") || value.starts_with("wss://"),
        "ASTERION_SERVER_URL 需要 HTTPS 源地址或 Agent 的完整 WSS 地址"
    );
    let mut url = Url::parse(value).context("ASTERION_SERVER_URL 必须是有效的 URL")?;
    ensure!(
        url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "ASTERION_SERVER_URL 仅接受中心地址"
    );

    match (url.scheme(), url.path()) {
        ("https", "/") => {
            url.set_scheme("wss")
                .map_err(|_| anyhow::anyhow!("无法转换中心连接地址"))?;
            url.set_path(AGENT_CONNECT_PATH);
        }
        ("wss", AGENT_CONNECT_PATH) => {}
        _ => bail!("ASTERION_SERVER_URL 需要 HTTPS 源地址或 Agent 的完整 WSS 地址"),
    }

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_the_secure_agent_endpoint() {
        let expected = format!("wss://center.example.invalid:8443{AGENT_CONNECT_PATH}");
        assert_eq!(
            agent_endpoint("https://center.example.invalid:8443/")
                .unwrap()
                .as_str(),
            expected
        );
        assert_eq!(agent_endpoint(&expected).unwrap().as_str(), expected);

        for value in [
            "http://center.example.invalid",
            "https:center.example.invalid",
            "ws://center.example.invalid/api/v1/agent/connect",
            "https://user:password@center.example.invalid",
            "https://center.example.invalid/?token=secret",
            "https://center.example.invalid/#fragment",
            "https://center.example.invalid/login",
            "wss://center.example.invalid/unknown",
        ] {
            assert!(agent_endpoint(value).is_err(), "{value}");
        }
    }
}
