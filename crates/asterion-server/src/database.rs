use std::fs::File;
#[cfg(unix)]
use std::fs::{OpenOptions, TryLockError};

use anyhow::{Context, Result, ensure};
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::config::DEFAULT_DATABASE_URL;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let pool = open(url).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// 持久库只能由一个中心使用。在 Unix 上使用与 SQLite 事务锁独立的文件锁。
pub async fn connect_center(url: &str) -> Result<(SqlitePool, Option<File>)> {
    let pool = open(url).await?;
    #[cfg(unix)]
    let guard = lock_database(&pool).await?;
    #[cfg(not(unix))]
    let guard = None;
    migrate(&pool).await?;
    Ok((pool, guard))
}

#[cfg(unix)]
async fn lock_database(pool: &SqlitePool) -> Result<Option<File>> {
    let path = sqlx::query_scalar::<_, String>(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
    )
    .fetch_one(pool)
    .await
    .context("无法确定中心 SQLite 文件")?;
    if path.is_empty() {
        return Ok(None);
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .context("无法打开中心 SQLite 文件以锁定")?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => anyhow::bail!("SQLite 数据库已由另一个中心进程使用"),
        Err(TryLockError::Error(error)) => Err(error).context("无法锁定中心 SQLite 数据库"),
    }
}

async fn open(url: &str) -> Result<SqlitePool> {
    ensure!(
        url.starts_with("sqlite:"),
        "ASTERION_DATABASE_URL 必须为 SQLite URL"
    );
    let options = url
        .parse::<SqliteConnectOptions>()
        .context("ASTERION_DATABASE_URL 无法解析")?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);

    if url == DEFAULT_DATABASE_URL {
        tokio::fs::create_dir_all(".local")
            .await
            .context("无法创建本地数据目录")?;
    }

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("无法打开 SQLite 数据库")
}

pub async fn connect_path(path: &std::path::Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    MIGRATOR.run(pool).await.context("SQLite 迁移失败")?;
    Ok(())
}
