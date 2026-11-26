mod config;
mod mounts;
mod sample;
mod storage;
#[cfg(test)]
mod storage_tests;

use anyhow::{Context, Result, bail};
use asterion_protocol::{TaskReply, collector::Cursor, unix_time};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{self, BufRead, BufReader, Read},
    time::Duration,
};

#[derive(Deserialize)]
struct ExportRequest {
    args: Cursor,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let path = config::database();
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [cmd, since] if cmd == "ready" => {
            storage::ready(&path, since.parse().context("采样时间无效")?).await
        }
        [cmd] if cmd == "--version" => {
            println!("asterion-collector {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [cmd] if cmd == "run" => {
            let policy = config::Policy::load()?;
            let now = || unix_time().context("系统时间无效");
            let mut db = storage::writer(&path, &policy, now()?).await?;
            let mut sampler = sample::Sampler::default();
            let mut timer = tokio::time::interval(Duration::from_secs(policy.interval));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let value = sampler.sample(now()?)?;
                storage::write(&mut db, &policy, &value).await?;
            }
        }
        [cmd] if cmd == "export" => {
            let mut line = String::new();
            BufReader::new(io::stdin().lock().take(520001)).read_line(&mut line)?;
            let request: ExportRequest = serde_json::from_str(&line)?;
            let reply = match storage::export(&path, &request.args).await {
                Ok(batch) => TaskReply::Success {
                    value: json!({"collector":batch}),
                },
                Err(error) => {
                    eprintln!("读取采集器历史失败：{error:#}");
                    TaskReply::failure(
                        "collector_unavailable",
                        "无法读取采集器数据库，请检查服务状态和 SSH 账号的读取权限",
                    )
                }
            };
            println!("{}", serde_json::to_string(&reply)?);
            Ok(())
        }
        _ => bail!("用法：asterion-collector run 或 asterion-collector export"),
    }
}
