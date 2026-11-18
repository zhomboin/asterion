use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use asterion_protocol::{is_node_id, is_token};
use serde::{Deserialize, Serialize};

const IDENTITY_LIMIT: usize = 4096;
const ENROLLMENT_LIMIT: usize = 128;
const SECURE_FLAGS: i32 = libc::O_NOFOLLOW | libc::O_NONBLOCK;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub endpoint: String,
    pub bearer: String,
    pub node_id: Option<String>,
}

impl Identity {
    pub fn new(endpoint: &str) -> Result<Self> {
        Ok(Self {
            endpoint: endpoint.to_owned(),
            bearer: random_hex()?,
            node_id: None,
        })
    }
}

pub struct StateStore {
    path: PathBuf,
    directory: PathBuf,
    _lock: File,
}

impl StateStore {
    pub fn open(path: &Path) -> Result<Self> {
        let name = path.file_name().context("身份文件路径缺少文件名")?;
        let parent = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory = fs::canonicalize(parent).context("身份数据目录必须预先创建")?;
        let metadata = fs::metadata(&directory).context("无法检查身份数据目录")?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == effective_uid()
                && metadata.mode() & 0o7777 == 0o700,
            "身份数据目录必须归 Agent 用户所有，且权限为 0700"
        );

        let mut lock_name = name.to_os_string();
        lock_name.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(SECURE_FLAGS)
            .open(directory.join(lock_name))
            .context("无法打开身份锁文件")?;
        ensure_private_file(&lock)?;
        lock.try_lock()
            .context("无法锁定 Agent 身份，请检查是否已启动")?;

        let path = directory.join(name);
        Ok(Self {
            path,
            directory,
            _lock: lock,
        })
    }

    pub fn load(&self, endpoint: &str) -> Result<Option<Identity>> {
        let Some(bytes) =
            read_private(&self.path, IDENTITY_LIMIT).context("无法读取 Agent 身份")?
        else {
            return Ok(None);
        };
        let identity: Identity =
            serde_json::from_slice(&bytes).map_err(|_| anyhow!("Agent 身份文件格式无效"))?;
        ensure!(identity.endpoint == endpoint, "Agent 身份与中心地址不匹配");
        ensure!(
            is_token(&identity.bearer) && identity.node_id.as_deref().is_none_or(is_node_id),
            "Agent 身份文件内容无效"
        );
        Ok(Some(identity))
    }

    pub fn save(&self, identity: &Identity) -> Result<()> {
        let temporary = self
            .directory
            .join(format!(".asterion-agent-{}.tmp", random_hex()?));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .context("无法创建临时身份文件")?;

        let written = (|| -> Result<()> {
            serde_json::to_writer(&mut file, identity).context("无法写入 Agent 身份")?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            File::open(&self.directory)?.sync_all()?;
            Ok(())
        })();

        if written.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        written.context("无法保存 Agent 身份")
    }
}

pub fn read_enrollment(path: &Path) -> Result<Option<String>> {
    let Some(bytes) = read_private(path, ENROLLMENT_LIMIT).context("无法读取 Agent 注册凭证")?
    else {
        return Ok(None);
    };
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow!("Agent 注册凭证格式无效"))?
        .trim();
    ensure!(is_token(value), "Agent 注册凭证格式无效");
    Ok(Some(value.to_owned()))
}

fn read_private(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(SECURE_FLAGS)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure_private_file(&file)?;

    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "凭证文件超过允许大小");
    Ok(Some(bytes))
}

fn ensure_private_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == effective_uid()
            && matches!(metadata.mode() & 0o7777, 0o400 | 0o600),
        "凭证文件必须由 Agent 用户持有，且权限为 0400 或 0600"
    );
    Ok(())
}

fn effective_uid() -> u32 {
    // geteuid has no arguments or preconditions and only returns process metadata.
    unsafe { libc::geteuid() }
}

fn random_hex() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| anyhow!("无法生成 Agent 身份凭证"))?;
    let mut hex = String::with_capacity(64);
    for value in bytes {
        write!(&mut hex, "{value:02x}").expect("writing to a String cannot fail");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};

    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("asterion-agent-{}", random_hex().unwrap()));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn persists_identity_and_locks_the_state_path() {
        let temp = Fixture::new();
        let path = temp.0.join("identity.json");
        let endpoint = "wss://center.example.invalid/api/v1/agent/connect";
        {
            let store = StateStore::open(&path).unwrap();
            assert!(store.load(endpoint).unwrap().is_none());
            assert!(StateStore::open(&path).is_err());
            let mut identity = Identity::new(endpoint).unwrap();
            identity.node_id = Some("a".repeat(32));
            store.save(&identity).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            assert_eq!(
                store.load(endpoint).unwrap().unwrap().bearer,
                identity.bearer
            );
            assert!(store.load("wss://other.example.invalid").is_err());
        }

        let store = StateStore::open(&path).unwrap();
        assert_eq!(
            store.load(endpoint).unwrap().unwrap().node_id,
            Some("a".repeat(32))
        );
    }

    #[test]
    fn denies_reading_insecure_secrets() {
        let temp = Fixture::new();
        let path = temp.0.join("registration");
        let link = temp.0.join("link");
        let token = "b".repeat(64);
        fs::write(&path, &token).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_enrollment(&path).is_err());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_enrollment(&path).unwrap(), Some(token));
        symlink(&path, &link).unwrap();
        assert!(read_enrollment(&link).is_err());

        fs::write(&path, "b".repeat(ENROLLMENT_LIMIT + 1)).unwrap();
        assert!(read_enrollment(&path).is_err());
        fs::remove_file(&path).unwrap();
        assert!(read_enrollment(&path).unwrap().is_none());
    }
}
