use anyhow::{Result, ensure};
use asterion_protocol::{
    MAX_SAMPLE_INTERVAL_SECS,
    collector::{BATCH_ROWS, Batch, Cursor, metrics_sample},
    is_node_id,
};
use sqlx::SqlitePool;

pub(super) async fn cursor(pool: &SqlitePool, node: &str) -> Result<Cursor, sqlx::Error> {
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT stream, seq FROM collector_sync WHERE node_id=? AND active=1")
            .bind(node)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .map(|(stream, seq)| Cursor { stream, seq })
        .unwrap_or_default())
}

pub(super) async fn ingest(
    pool: &SqlitePool,
    node: &str,
    requested: &Cursor,
    batch: Batch,
    period: u64,
    retention: (u64, u64),
) -> Result<bool> {
    ensure!(
        is_node_id(&batch.stream)
            && batch.newest_seq >= 0
            && batch.records.len() <= BATCH_ROWS
            && (5..=MAX_SAMPLE_INTERVAL_SECS).contains(&batch.interval_secs)
            && !batch.arch.is_empty()
            && batch.arch.len() <= 64
            && batch.newest_at.is_none_or(|at| at >= 0)
            && batch.newest_at.is_some() == (batch.newest_seq > 0),
        "采集器响应格式无效"
    );
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let state: Option<(String, i64)> =
        sqlx::query_as("SELECT stream,seq FROM collector_sync WHERE node_id=? AND active=1")
            .bind(node)
            .fetch_optional(&mut *tx)
            .await?;
    let previous = state
        .map(|(stream, seq)| Cursor { stream, seq })
        .unwrap_or_default();
    if previous != *requested {
        return Ok(false);
    }
    let now: i64 = sqlx::query_scalar("SELECT unixepoch()")
        .fetch_one(&mut *tx)
        .await?;
    let raw_cutoff = now - retention.0 as i64 * 86400;
    let trend_cutoff = now - retention.1 as i64 * 86400;
    let mut seq = if previous.stream == batch.stream {
        previous.seq
    } else {
        sqlx::query_scalar::<_, i64>("SELECT seq FROM collector_sync WHERE node_id=? AND stream=?")
            .bind(node)
            .bind(&batch.stream)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(0)
    };
    let mut wire_seq = if previous.stream == batch.stream {
        previous.seq
    } else {
        0
    };
    let start = seq;
    let mut missed = 0;
    let mut waiting_for_clock = false;
    for row in batch.records {
        ensure!(
            row.seq > wire_seq && row.seq <= batch.newest_seq,
            "采集器序号无效"
        );
        wire_seq = row.seq;
        if row.seq <= seq {
            continue;
        }
        let encoded =
            metrics_sample(&row.sample).ok_or_else(|| anyhow::anyhow!("采集器指标格式无效"))?;
        let at = row.sample["captured_at"].as_i64().unwrap();
        // 小幅时钟偏差等到下轮再读，以免刚采下的样本被误丢弃。
        if at > now && at <= now + 300 {
            waiting_for_clock = true;
            break;
        }
        missed += row.seq - seq - 1;
        seq = row.seq;
        // 时钟错误的样本不能进入未来的图表，也不能阻塞后续有效记录。
        if at > now {
            missed += 1;
            continue;
        }
        let keep_trend = at.div_euclid(300) * 300 >= trend_cutoff;
        let fresh = if at >= raw_cutoff {
            sqlx::query(
                "INSERT INTO metric_samples(node_id,observed_at,sample) VALUES(?,?,?)
                ON CONFLICT(node_id,observed_at) DO NOTHING",
            )
            .bind(node)
            .bind(at)
            .bind(encoded)
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        } else if keep_trend {
            !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM metric_samples WHERE node_id=? AND observed_at=?)",
            )
            .bind(node)
            .bind(at)
            .fetch_one(&mut *tx)
            .await?
        } else {
            false
        };
        if fresh && keep_trend {
            crate::trends::observe(&mut tx, node, at, &row.sample).await?;
        }
    }
    ensure!(
        batch.newest_seq == 0 || seq <= batch.newest_seq,
        "采集器数据库序号回退"
    );
    ensure!(
        waiting_for_clock
            || previous.stream != batch.stream
            || seq == batch.newest_seq
            || seq > start
            || batch.newest_seq == 0,
        "采集器分页没有推进"
    );
    if previous.stream != batch.stream {
        sqlx::query("UPDATE collector_sync SET active=0 WHERE node_id=? AND active=1")
            .bind(node)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO collector_sync(node_id,stream,seq,newest_seq,interval_secs,missed) VALUES(?,?,?,?,?,?)
        ON CONFLICT(node_id,stream) DO UPDATE SET active=1,seq=excluded.seq,newest_seq=excluded.newest_seq,
            interval_secs=excluded.interval_secs,missed=collector_sync.missed+excluded.missed")
        .bind(node).bind(&batch.stream).bind(seq).bind(batch.newest_seq).bind(batch.interval_secs as i64)
        .bind(missed).execute(&mut *tx).await?;
    let warning = match batch.newest_at {
        Some(at) if at > now => Some("采集器时间领先于面板，请校准系统时间"),
        Some(at) if now - at <= batch.interval_secs as i64 * 2 + 15 => None,
        _ => Some("采集器尚无近期数据，请检查远端 systemd 服务"),
    };
    sqlx::query("UPDATE nodes SET ssh_online_until=?+?, last_seen_at=?, os='Linux',arch=?,ssh_error=? WHERE id=?")
        .bind(now).bind((period*2+15) as i64).bind(now).bind(&batch.arch).bind(warning)
        .bind(node).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(!waiting_for_clock && seq < batch.newest_seq)
}
