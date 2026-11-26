use super::collector::{cursor, ingest};
use asterion_protocol::collector::{Batch, Cursor, Record};
use serde_json::json;

fn batch(stream: &str, newest: i64, points: &[(i64, i64)]) -> Batch {
    let records = points.iter().map(|&(seq,at)| Record{seq,sample:json!({
        "captured_at":at,"logical_cores":1,"cpu_percent":50,"load":[0,0,0],
        "memory":{"total_bytes":2,"used_bytes":1,"available_bytes":1},
        "swap":{"total_bytes":0,"used_bytes":0,"available_bytes":0},"interfaces":[],"filesystems":[]
    })}).collect();
    Batch {
        stream: stream.into(),
        newest_seq: newest,
        newest_at: Some(asterion_protocol::unix_time().unwrap()),
        records,
        interval_secs: 15,
        arch: "x86_64".into(),
    }
}

#[tokio::test]
async fn backfill_is_atomic_idempotent_and_respects_both_retention_windows() -> anyhow::Result<()> {
    let pool = crate::database::connect("sqlite::memory:").await?;
    let node = "0".repeat(32);
    let stream = "1".repeat(32);
    sqlx::query("INSERT INTO nodes(id,name,kind) VALUES(?,'采集器测试','linux')")
        .bind(&node)
        .execute(&pool)
        .await?;
    let now = asterion_protocol::unix_time().unwrap();
    let points = [
        (3, now - 100 * 86400),
        (4, now - 8 * 86400),
        (5, now - 600),
        (6, now + 86400),
    ];
    let start = Cursor::default();
    let retain = (7, 90);
    let mut invalid = batch(&stream, 7, &points);
    invalid.records[2].sample = json!({});
    assert!(
        ingest(&pool, &node, &start, invalid, 15, retain)
            .await
            .is_err()
    );
    assert_eq!(cursor(&pool, &node).await?, start);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM metric_rollups")
            .fetch_one(&pool)
            .await?,
        0
    );
    assert!(ingest(&pool, &node, &start, batch(&stream, 7, &points), 15, retain).await?);
    assert!(!ingest(&pool, &node, &start, batch(&stream, 7, &points), 15, retain).await?);
    let committed = cursor(&pool, &node).await?;
    assert_eq!(committed.seq, 6);
    assert!(
        !ingest(
            &pool,
            &node,
            &committed,
            batch(&stream, 7, &[(7, now - 60)]),
            15,
            retain
        )
        .await?
    );
    let next_stream = "2".repeat(32);
    let committed = cursor(&pool, &node).await?;
    assert!(
        !ingest(
            &pool,
            &node,
            &committed,
            batch(&next_stream, 1, &[(1, now - 60)]),
            15,
            retain
        )
        .await?
    );
    let switched = cursor(&pool, &node).await?;
    assert!(
        !ingest(
            &pool,
            &node,
            &switched,
            batch(&stream, 7, &points),
            15,
            retain
        )
        .await?
    );
    let restored = cursor(&pool, &node).await?;
    assert_eq!(restored.seq, 7);
    assert!(
        !ingest(
            &pool,
            &node,
            &restored,
            batch(&stream, 8, &[(8, now + 60)]),
            15,
            retain
        )
        .await?
    );
    assert_eq!(cursor(&pool, &node).await?, restored);

    let rows: Vec<i64> =
        sqlx::query_scalar("SELECT observed_at FROM metric_samples ORDER BY observed_at")
            .fetch_all(&pool)
            .await?;
    assert_eq!(rows, vec![now - 600, now - 60]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(sample_count) FROM metric_rollups")
            .fetch_one(&pool)
            .await?,
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(missed) FROM collector_sync")
            .fetch_one(&pool)
            .await?,
        3
    );
    Ok(())
}
