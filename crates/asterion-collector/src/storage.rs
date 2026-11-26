use crate::config::Policy;
use anyhow::{Context, Result, ensure};
use asterion_protocol::collector::{
    BATCH_BYTES, BATCH_ROWS, Batch, Cursor, Record, metrics_sample,
};
use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::{
    Connection, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{path::Path, time::Duration};

pub async fn writer(path: &Path, policy: &Policy, now: i64) -> Result<SqliteConnection> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete)
        .synchronous(SqliteSynchronous::Full)
        .pragma("auto_vacuum", "FULL")
        .pragma("cache_spill", "OFF")
        .busy_timeout(Duration::from_secs(5));
    let mut db = SqliteConnection::connect_with(&options).await?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS metadata (
        id INTEGER PRIMARY KEY CHECK(id=1), stream TEXT NOT NULL, interval_secs INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS readings (
        seq INTEGER PRIMARY KEY AUTOINCREMENT, captured_at INTEGER NOT NULL UNIQUE, sample TEXT NOT NULL);")
        .execute(&mut db).await?;
    sqlx::query(
        "INSERT INTO metadata VALUES (1,lower(hex(randomblob(16))),?) ON CONFLICT(id)
        DO UPDATE SET interval_secs=excluded.interval_secs",
    )
    .bind(policy.interval as i64)
    .execute(&mut db)
    .await?;
    clean(&mut db, now, policy, 0).await?;
    let limit = page_limit(&mut db, policy).await?;
    sqlx::query(&format!("PRAGMA max_page_count={limit}"))
        .execute(&mut db)
        .await?;
    Ok(db)
}

async fn page_limit(db: &mut SqliteConnection, policy: &Policy) -> Result<i64> {
    let page: i64 = sqlx::query_scalar("PRAGMA page_size").fetch_one(db).await?;
    // 回滚日志最多保存每页的原像、页号和校验和。关闭 spill，仅需预留一个日志头。
    Ok((policy.max_bytes - 65536) / (page * 2 + 8))
}

async fn clean(db: &mut SqliteConnection, now: i64, policy: &Policy, reserve: i64) -> Result<()> {
    let expired = now.saturating_sub(policy.retention);
    loop {
        let count = sqlx::query(
            "DELETE FROM readings WHERE seq IN
            (SELECT seq FROM readings WHERE captured_at < ? ORDER BY captured_at LIMIT 256)",
        )
        .bind(expired)
        .execute(&mut *db)
        .await?
        .rows_affected();
        if count < 256 {
            break;
        }
    }
    let limit = page_limit(db, policy).await?;
    loop {
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(&mut *db)
            .await?;
        if pages + reserve <= limit {
            break;
        }
        let removed = sqlx::query("DELETE FROM readings WHERE seq IN (SELECT seq FROM readings ORDER BY captured_at LIMIT 16)")
            .execute(&mut *db).await?.rows_affected();
        ensure!(removed > 0, "缓存容量不足以保存一个样本");
    }
    Ok(())
}

pub async fn write(db: &mut SqliteConnection, policy: &Policy, sample: &Value) -> Result<()> {
    let encoded = metrics_sample(sample).context("主机采样格式无效")?;
    let now = sample["captured_at"].as_i64().unwrap();
    // 数据页、溢出页以及两个 B 树的分裂空间；SQLite 同时实施硬页数上限。
    let reserve = 10 + (encoded.len() as i64 / 4000);
    clean(db, now, policy, reserve).await?;
    sqlx::query(
        "INSERT INTO readings(captured_at,sample) SELECT ?,? WHERE NOT EXISTS
        (SELECT 1 FROM readings WHERE captured_at=?)",
    )
    .bind(now)
    .bind(encoded)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

async fn reader(path: &Path) -> Result<SqliteConnection> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(5));
    Ok(SqliteConnection::connect_with(&options).await?)
}

pub async fn ready(path: &Path, since: i64) -> Result<()> {
    let mut db = reader(path).await?;
    let newest: Option<i64> =
        sqlx::query_scalar("SELECT captured_at FROM readings ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut db)
            .await?;
    ensure!(
        newest.is_some_and(|at| at >= since && at <= asterion_protocol::unix_time().unwrap_or(0)),
        "尚未读到本次服务启动后的采样"
    );
    Ok(())
}

pub async fn export(path: &Path, cursor: &Cursor) -> Result<Batch> {
    let mut db = reader(path).await?;
    let mut tx = db.begin().await?;
    let (stream, interval_secs): (String, i64) =
        sqlx::query_as("SELECT stream,interval_secs FROM metadata WHERE id=1")
            .fetch_one(&mut *tx)
            .await?;
    let after = if cursor.stream == stream {
        cursor.seq
    } else {
        0
    };
    let latest: Option<(i64, i64)> =
        sqlx::query_as("SELECT seq,captured_at FROM readings ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    let mut rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT seq,sample FROM readings WHERE seq>? ORDER BY seq LIMIT ?",
    )
    .bind(after)
    .bind(BATCH_ROWS as i64)
    .fetch(&mut *tx);
    let mut records = Vec::new();
    let mut bytes = 0;
    while let Some((seq, encoded)) = rows.try_next().await? {
        bytes += encoded.len() + 64;
        if bytes > BATCH_BYTES {
            break;
        }
        records.push(Record {
            seq,
            sample: serde_json::from_str(&encoded)?,
        });
    }
    drop(rows);
    tx.commit().await?;
    Ok(Batch {
        stream,
        records,
        newest_seq: latest.map_or(0, |r| r.0),
        newest_at: latest.map(|r| r.1),
        interval_secs: interval_secs.try_into()?,
        arch: std::env::consts::ARCH.to_owned(),
    })
}
