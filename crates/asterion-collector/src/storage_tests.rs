use crate::{config::Policy, storage};
use asterion_protocol::collector::Cursor;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> anyhow::Result<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local");
        fs::create_dir_all(&root)?;
        let path = root.join(format!(
            "collector-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn cache_survives_restart_and_enforces_age_and_disk_budget() -> anyhow::Result<()> {
    let temp = Temp::new()?;
    let path = temp.0.join("history.db");
    let mut policy = Policy {
        interval: 15,
        retention: 86400,
        max_bytes: 1024 * 1024,
    };
    let mut db = storage::writer(&path, &policy, 100).await?;
    let mut sample = crate::sample::Sampler::default().sample(100)?;
    let cpu = 15.547949981178155_f64;
    sample["cpu_percent"] = json!(cpu);
    let mount = format!("/{}", "x".repeat(4000));
    sample["filesystems"] =
        json!([{"mountpoint":mount,"fs_type":"ext4","total_bytes":2,"available_bytes":1}]);
    for i in 0..160 {
        sample["captured_at"] = json!(100 + i);
        storage::write(&mut db, &policy, &sample).await?;
    }
    let first = storage::export(&path, &Cursor::default()).await?;
    assert!(first.records[0].seq > 1);
    assert_eq!(first.records[0].sample["cpu_percent"].as_f64(), Some(cpu));
    assert!(first.records.last().unwrap().seq < first.newest_seq);
    assert_eq!(first.newest_seq, 160);
    assert!(fs::metadata(&path)?.len() < policy.max_bytes as u64);
    let cursor = Cursor {
        stream: first.stream.clone(),
        seq: first.records.last().unwrap().seq,
    };
    let next = storage::export(&path, &cursor).await?;
    assert_eq!(next.records[0].seq, cursor.seq + 1);
    drop(db);
    policy.retention = 4;
    let mut db = storage::writer(&path, &policy, 259).await?;
    storage::write(&mut db, &policy, &sample).await?;
    let retained = storage::export(&path, &Cursor::default()).await?;
    assert_eq!(retained.stream, first.stream);
    assert_eq!(retained.newest_seq, 160);
    assert_eq!(retained.records.len(), 5);
    assert_eq!(retained.records[0].sample["captured_at"], 255);
    let done = Cursor {
        stream: first.stream,
        seq: 160,
    };
    assert!(storage::export(&path, &done).await?.records.is_empty());
    let reset = storage::export(
        &path,
        &Cursor {
            stream: "other".into(),
            seq: 999,
        },
    )
    .await?;
    assert_eq!(reset.records.len(), 5);
    Ok(())
}
