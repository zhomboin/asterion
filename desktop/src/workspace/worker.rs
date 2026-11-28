use super::remote::Remote;
use asterion_protocol::{TaskReply, unix_time};
use asterion_server::{
    collector_deploy, samples,
    ssh_bus::{Control, Dispatch, SshBus},
};
use serde_json::json;
use sqlx::SqlitePool;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::oneshot,
    task::{AbortHandle, JoinSet},
    time::{MissedTickBehavior, interval},
};

pub struct Running(pub AbortHandle);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn start(pool: SqlitePool, bus: &Arc<SshBus>, id: String, remote: Remote) -> Running {
    let mut requests = bus.attach(&id, true);
    let handle = tokio::spawn(async move {
        let mut timer = interval(Duration::from_secs(15));
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut jobs = JoinSet::new();
        let mut cancels = HashMap::<String, oneshot::Sender<()>>::new();
        let mut sampling = false;
        loop {
            tokio::select! {
                _=timer.tick(), if !sampling => {
                    if remote.session.is_closed() { break; }
                    sampling=true;
                    let (pool,id,remote)=(pool.clone(),id.clone(),remote.clone());
                    jobs.spawn(async move {
                        if sync(&pool,&id,&remote).await.is_err() {
                            let _=samples::save_sample(&pool,&id,15,TaskReply::failure("offline","采样中断")).await;
                        }
                        None
                    });
                }
                Some(result)=jobs.join_next(), if !jobs.is_empty() => {
                    match result {Ok(Some(id))=>{cancels.remove(&id);},Ok(None)=>sampling=false,Err(_)=>break}
                }
                command=requests.next()=>match command {
                    Control::Cancel(id)=>{if let Some(tx)=cancels.remove(&id) {let _=tx.send(());}}
                    Control::Task(task)=>{
                        if jobs.len()>=9 {let _=task.response.send(TaskReply::failure("busy","节点任务繁忙"));continue;}
                        let (tx,rx)=oneshot::channel();
                        let key=task.request.request_id.clone();
                        cancels.insert(key.clone(),tx);
                        let (remote,pool)=(remote.clone(),pool.clone());
                        jobs.spawn(async move {
                            dispatch(remote,pool,task,rx).await;
                            Some(key)
                        });
                    }
                }
            }
        }
    });
    Running(handle.abort_handle())
}

async fn dispatch(remote: Remote, pool: SqlitePool, task: Dispatch, cancel: oneshot::Receiver<()>) {
    let Dispatch {
        request,
        response,
        progress,
    } = task;
    let seconds = request
        .expires_at
        .saturating_sub(unix_time().unwrap_or(i64::MAX))
        .max(0) as u64;
    let action = async {
        if seconds == 0 {
            return TaskReply::failure("timeout", "请求已过期");
        }
        if request.kind == "collector_status" {
            return collector_deploy::status(&remote).await;
        }
        if request.kind == "operation_execute" {
            let running=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM operation_jobs WHERE id=? AND state='running' AND cancel_requested_at IS NULL)")
                .bind(&request.request_id).fetch_one(&pool).await;
            if !matches!(running, Ok(true)) {
                return TaskReply::failure("cancelled", "任务已停止");
            }
            if request.args["action"]["type"] == "collector_deploy" {
                return collector_deploy::deploy(&remote, request.args, &progress).await;
            }
            if request.args.get("sudo_password").is_some() {
                return TaskReply::failure("invalid_argument", "此操作不接受 sudo 密码");
            }
        }
        remote
            .query(&request.kind, request.args, seconds, Some(&progress))
            .await
    };
    let reply = tokio::select! {
        r=tokio::time::timeout(Duration::from_secs(seconds),action)=>r.unwrap_or_else(|_|TaskReply::failure("timeout","SSH 请求超时")),
        _=cancel=>TaskReply::failure("cancelled","请求已取消"),
    };
    let _ = response.send(reply);
}

async fn sync(pool: &SqlitePool, id: &str, remote: &Remote) -> anyhow::Result<()> {
    for _ in 0..8 {
        let cursor = samples::cursor(pool, id).await?;
        match remote.query("sample", json!(cursor), 10, None).await {
            TaskReply::Success { mut value } if value.get("collector").is_some() => {
                let batch = serde_json::from_value(value["collector"].take())?;
                if !samples::ingest(pool, id, &cursor, batch, 15, (7, 90)).await? {
                    break;
                }
            }
            reply => {
                samples::save_sample(pool, id, 15, reply).await?;
                break;
            }
        }
    }
    Ok(())
}
