use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, ensure};
use sqlx::{Connection, SqliteConnection, SqlitePool, sqlite::SqliteConnectOptions};
use tokio::process::Command;

const ASKPASS_REVISION: &str = "ASTERION_ASKPASS_REVISION";
const ASKPASS_DB: &str = "ASTERION_ASKPASS_DB";
pub(super) const ASKPASS_OWNER: &str = "ASTERION_ASKPASS_OWNER";

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum Auth {
    #[default]
    Default,
    Key(String),
    Password {
        database: PathBuf,
        revision: String,
    },
    Paused,
}

impl Auth {
    pub(super) fn configure(&self, command: &mut Command) {
        match self {
            Self::Key(file) => {
                command.args([
                    "-oIdentitiesOnly=yes",
                    "-oPreferredAuthentications=publickey",
                    "-i",
                    file,
                ]);
            }
            Self::Password { database, revision } => {
                // OpenSSH 仅将密码提示交给本机助手。子进程环境中只传凭据标识。
                command
                    .args([
                        "-oBatchMode=no",
                        "-oPreferredAuthentications=password",
                        "-oPasswordAuthentication=yes",
                        "-oNumberOfPasswordPrompts=1",
                    ])
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env(
                        "SSH_ASKPASS",
                        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/dev/null")),
                    )
                    .env("LC_ALL", "C")
                    .env(ASKPASS_OWNER, std::process::id().to_string())
                    .env(ASKPASS_REVISION, revision)
                    .env(ASKPASS_DB, database);
            }
            Self::Default | Self::Paused => {}
        }
    }
}

#[derive(sqlx::FromRow)]
struct Stored {
    node_id: String,
    ssh_profile: String,
    method: Option<String>,
    identity_file: Option<String>,
    revision: Option<String>,
    paused: Option<bool>,
}

pub(super) async fn load(
    pool: &SqlitePool,
    database: &std::path::Path,
) -> Result<Vec<(String, String, Auth)>> {
    let rows = sqlx::query_as::<_, Stored>(
        "SELECT n.id AS node_id, n.ssh_profile, a.method, a.identity_file, a.revision, a.paused
         FROM nodes n LEFT JOIN node_ssh_auth a ON a.node_id=n.id WHERE n.ssh_profile IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let auth = match (
                row.method.as_deref(),
                row.identity_file,
                row.revision,
                row.paused,
            ) {
                (None, _, _, _) => Auth::Default,
                (Some("password"), _, Some(revision), Some(false)) => Auth::Password {
                    database: database.into(),
                    revision,
                },
                (Some("key"), Some(file), _, Some(false)) => Auth::Key(file),
                _ => Auth::Paused,
            };
            (row.node_id, row.ssh_profile, auth)
        })
        .collect())
}

pub fn is_askpass() -> bool {
    std::env::var_os(ASKPASS_REVISION).is_some()
}

pub(super) async fn pause_password(
    pool: &SqlitePool,
    id: &str,
    revision: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let updated = sqlx::query("UPDATE node_ssh_auth SET paused=1 WHERE node_id=? AND revision=?")
        .bind(id)
        .bind(revision)
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() > 0 {
        sqlx::query("UPDATE nodes SET ssh_online_until=unixepoch(), ssh_error=? WHERE id=?")
            .bind("SSH 密码登录被拒绝，已暂停重试。请在管理节点中更新密码，确认服务器已启用密码认证。")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await
}

pub(super) fn verify_askpass_owner() -> Result<()> {
    // 跳板机启动的第二个 ssh 也可能继承助手环境，只允许面板直接启动的客户端取密码。
    let ssh_pid = unsafe { libc::getppid() };
    let status = std::fs::read_to_string(format!("/proc/{ssh_pid}/status"))?;
    let owner = std::env::var(ASKPASS_OWNER)?;
    ensure!(
        status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .map(str::trim)
            == Some(owner.as_str()),
        "SSH 请求来源无效"
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
pub async fn askpass() -> Result<()> {
    verify_askpass_owner()?;
    let prompt = std::env::args_os().nth(1).unwrap_or_default();
    ensure!(
        prompt.to_string_lossy().trim_end().ends_with("password:")
            && std::env::var_os("SSH_ASKPASS_PROMPT").is_none(),
        "无效的 SSH 密码提示"
    );
    let path = std::env::var_os(ASKPASS_DB).ok_or_else(|| anyhow::anyhow!("缺少 SSH 凭据位置"))?;
    let revision = std::env::var(ASKPASS_REVISION)?;
    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    let mut db = SqliteConnection::connect_with(&options).await?;
    let password: String = sqlx::query_scalar(
        "SELECT password FROM node_ssh_auth WHERE revision=? AND method='password' AND paused=0",
    )
    .bind(revision)
    .fetch_one(&mut db)
    .await?;
    writeln!(std::io::stdout().lock(), "{password}")?;
    Ok(())
}
