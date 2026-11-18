use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use rustls::{ClientConfig, RootCertStore};

const CA_FILE_LIMIT: u64 = 1024 * 1024;

pub fn client_config(custom_ca: Option<&Path>) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    roots.add_parsable_certificates(native.certs);

    if let Some(path) = custom_ca {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .context("无法打开 Agent CA 文件")?;
        let metadata = file.metadata().context("无法检查 Agent CA 文件")?;
        ensure!(
            metadata.is_file() && metadata.len() <= CA_FILE_LIMIT,
            "Agent CA 文件必须为不超过 1 MiB 的 PEM 普通文件"
        );

        let mut input = Vec::new();
        file.take(CA_FILE_LIMIT + 1).read_to_end(&mut input)?;
        ensure!(
            input.len() as u64 <= CA_FILE_LIMIT,
            "Agent CA 文件超过 1 MiB"
        );
        let mut found = false;
        for certificate in rustls_pemfile::certs(&mut input.as_slice()) {
            roots
                .add(certificate.context("Agent CA 文件包含无效的 PEM 证书")?)
                .context("无法将证书加入 Agent 信任根")?;
            found = true;
        }
        ensure!(found, "Agent CA 文件至少需要一张 PEM 证书");
    }

    ensure!(!roots.is_empty(), "Agent 未找到可用的 TLS 信任根");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}
