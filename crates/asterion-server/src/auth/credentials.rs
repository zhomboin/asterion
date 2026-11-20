use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use argon2::Argon2;
use argon2::password_hash::{Error, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use sqlx::SqlitePool;

use super::AuthError;

pub async fn bootstrap_admin(
    pool: &SqlitePool,
    username: Option<&str>,
    password_file: Option<&Path>,
) -> Result<()> {
    if let Some(username) = username {
        ensure!(
            valid_username(username),
            "ASTERION_ADMIN_USERNAME 必须是 1 到 64 个允许的 ASCII 字符"
        );
    }
    if let Some(existing) = configured_admin(pool).await? {
        return verify_bootstrap_username(username, &existing);
    }

    let username = username.context("首次启动需要配置 ASTERION_ADMIN_USERNAME")?;
    let path =
        PathBuf::from(password_file.context("首次启动需要配置 ASTERION_ADMIN_PASSWORD_FILE")?);
    let hash = tokio::task::spawn_blocking(move || hash_password_file(&path)).await??;
    insert_admin(pool, username, hash).await
}

pub async fn bootstrap_local_admin(pool: &SqlitePool) -> Result<()> {
    if configured_admin(pool).await?.is_some() {
        return Ok(());
    }
    let hash = tokio::task::spawn_blocking(|| -> Result<String> {
        let script = r#"exec </dev/tty || exit 1
IFS= read -rs -p '首次启动，请设置本机面板口令（12 至 128 字节） ' password || exit 1
printf '\n' >&2
IFS= read -rs -p '请再次输入口令 ' confirm || exit 1
printf '\n' >&2
[[ "$password" == "$confirm" ]] || { echo '两次口令不同' >&2; exit 1; }
printf '%s' "$password""#;
        let output = std::process::Command::new("/bin/bash")
            .args(["-c", script])
            .stderr(std::process::Stdio::inherit())
            .output()?;
        ensure!(output.status.success(), "请从本机交互终端启动并设置口令");
        let password = String::from_utf8(output.stdout)?;
        ensure!(
            valid_password(&password),
            "口令必须为 12 至 128 UTF-8 字节的单行文本"
        );
        hash_password(&password)
    })
    .await??;
    insert_admin(pool, "local", hash).await
}

async fn insert_admin(pool: &SqlitePool, username: &str, hash: String) -> Result<()> {
    let inserted = sqlx::query(
        "INSERT INTO panel_admin (id, username, password_hash) VALUES (1, ?, ?)
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(username)
    .bind(hash)
    .execute(pool)
    .await
    .context("无法建立面板管理员")?;

    if inserted.rows_affected() == 0 {
        let existing = configured_admin(pool).await?.context("面板管理员不存在")?;
        verify_bootstrap_username(Some(username), &existing)?;
    }
    Ok(())
}

async fn configured_admin(pool: &SqlitePool) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT username FROM panel_admin WHERE id = 1")
        .fetch_optional(pool)
        .await
        .context("无法读取面板管理员")
}

fn verify_bootstrap_username(username: Option<&str>, existing: &str) -> Result<()> {
    ensure!(
        username.is_none_or(|provided| provided == existing),
        "ASTERION_ADMIN_USERNAME 与 SQLite 中现有管理员不一致"
    );
    Ok(())
}

pub fn valid_username(username: &str) -> bool {
    (1..=64).contains(&username.len())
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
}

pub fn valid_password(password: &str) -> bool {
    (12..=128).contains(&password.len())
        && !password.contains(['\r', '\n', '\0'])
        && !password.chars().all(char::is_whitespace)
}

fn hash_password_file(path: &Path) -> Result<String> {
    let password = read_password(path)?;
    hash_password(&password)
}

fn hash_password(password: &str) -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| anyhow!("无法生成管理员口令盐"))?;
    let salt = SaltString::encode_b64(&bytes).context("无法编码管理员口令盐")?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .context("无法生成管理员口令哈希")?;
    Ok(hash.to_string())
}

fn read_password(path: &Path) -> Result<String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).context("无法打开管理员口令文件")?;
    let metadata = file.metadata().context("无法读取管理员口令文件属性")?;
    ensure!(metadata.is_file(), "管理员口令必须位于普通文件");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            matches!(metadata.permissions().mode() & 0o7777, 0o400 | 0o600),
            "管理员口令文件权限必须为 0400 或 0600"
        );
    }

    let mut password = String::new();
    file.take(131)
        .read_to_string(&mut password)
        .context("无法读取 UTF-8 管理员口令")?;
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    if !valid_password(&password) {
        bail!("管理员口令必须为 12 到 128 UTF-8 字节的单行文本");
    }
    Ok(password)
}

pub(super) fn verify_password(password: &str, password_hash: &str) -> Result<bool, AuthError> {
    let hash = PasswordHash::new(password_hash).map_err(AuthError::internal)?;
    match Argon2::default().verify_password(password.as_bytes(), &hash) {
        Ok(()) => Ok(true),
        Err(Error::Password) => Ok(false),
        Err(error) => Err(AuthError::internal(error)),
    }
}
