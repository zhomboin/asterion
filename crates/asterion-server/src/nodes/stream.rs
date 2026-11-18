use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use asterion_protocol::{
    AgentMessage, AgentReply, HEARTBEAT_TIMEOUT_SECS, ServerMessage, TaskProgress, is_node_id,
    unix_time,
};
use axum::extract::ws::WebSocket;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use crate::agent_bus::{AgentBus, Control, Dispatch, MAX_PENDING, Subscription};
use crate::monitor;

use super::credentials::{self, Connection};
use super::socket::{SocketFailure, receive, reply, validate_heartbeat};

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(HEARTBEAT_TIMEOUT_SECS);
const MIN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

pub(super) async fn run(
    socket: &mut WebSocket,
    pool: SqlitePool,
    connection: Connection,
    bus: Arc<AgentBus>,
    version: u16,
) -> Result<(), SocketFailure> {
    let can_monitor = version >= 2;
    let mut subscription = subscribe(&pool, &connection, &bus, can_monitor).await?;
    let mut outgoing = Outgoing {
        node_id: &connection.node_id,
        version,
        sequence: 1,
    };
    let sample_interval = bus.sample_interval_secs();
    if can_monitor {
        outgoing
            .send(
                socket,
                ServerMessage::Configure {
                    sample_interval_secs: sample_interval,
                },
            )
            .await?;
    }

    let mut pending = HashMap::<String, Updates>::new();
    let mut last_message_id = 1;
    let mut last_heartbeat = Instant::now();
    let mut last_sample = None::<Instant>;
    loop {
        tokio::select! {
            message = receive(socket, last_heartbeat + HEARTBEAT_TIMEOUT) => {
                let message = message?;
                if message.protocol_version != version
                    || message.timestamp < 0
                    || message.message_id <= last_message_id
                    || message.node_id.as_deref() != Some(connection.node_id.as_str())
                {
                    return Err(SocketFailure::Policy);
                }
                let previous = last_message_id;
                last_message_id = message.message_id;
                match message.message {
                    AgentMessage::Heartbeat {} => {
                        validate_heartbeat(&message, &connection.node_id, previous)?;
                        if last_heartbeat.elapsed() < MIN_HEARTBEAT_INTERVAL {
                            return Err(SocketFailure::Policy);
                        }
                        last_heartbeat = Instant::now();
                        let seen_at = credentials::heartbeat(&pool, &connection).await?;
                        reply(socket, &connection.node_id, version, last_message_id, seen_at,
                              ServerMessage::HeartbeatAck {}).await?;
                        cancel_abandoned(socket, &mut outgoing, &mut pending).await?;
                    }
                    AgentMessage::Metrics { sample } if can_monitor => {
                        let min_gap = Duration::from_secs(sample_interval.saturating_sub(1));
                        if last_sample.is_some_and(|last| last.elapsed() < min_gap) {
                            continue;
                        }
                        store(&pool, &connection, &sample).await?;
                        last_sample = Some(Instant::now());
                    }
                    AgentMessage::TaskReply { request_id, reply } if can_monitor => {
                        if !is_node_id(&request_id) || !valid_reply(&reply) {
                            return Err(SocketFailure::Policy);
                        }
                        if let Some(response) = pending.remove(&request_id) {
                            check_current(&pool, &connection).await?;
                            let _ = response.response.send(reply);
                        }
                    }
                    AgentMessage::TaskProgress { request_id, progress } if can_monitor => {
                        if !is_node_id(&request_id) || progress.output.len() > 16 * 1024 {
                            return Err(SocketFailure::Policy);
                        }
                        if let Some(updates) = pending.get_mut(&request_id)
                            && updates.last_progress.is_none_or(|last| last.elapsed() >= Duration::from_millis(200))
                        {
                            updates.last_progress = Some(Instant::now());
                            check_current(&pool, &connection).await?;
                            updates.progress.send_replace(Some(progress));
                        }
                    }
                    _ => return Err(SocketFailure::Protocol),
                }
            }
            control = subscription.next(), if can_monitor => {
                cancel_abandoned(socket, &mut outgoing, &mut pending).await?;
                match control {
                    Control::Task(dispatch) =>
                        forward(socket, &pool, &connection, &mut outgoing, &mut pending, dispatch).await?,
                    Control::Cancel(request_id) if pending.contains_key(&request_id) =>
                        outgoing.send(socket, ServerMessage::CancelTask { request_id }).await?,
                    Control::Cancel(_) => {}
                }
            }
        }
    }
}

pub(super) async fn subscribe(
    pool: &SqlitePool,
    connection: &Connection,
    bus: &Arc<AgentBus>,
    can_monitor: bool,
) -> Result<Subscription, SocketFailure> {
    // 序列化连接代次校验和路由安装，避免迟到的握手覆盖较新的连接。
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| SocketFailure::Internal)?;
    credentials::ensure_current(&mut transaction, connection).await?;
    let subscription = bus.attach(&connection.node_id, can_monitor);
    transaction
        .commit()
        .await
        .map_err(|_| SocketFailure::Internal)?;
    Ok(subscription)
}

async fn forward(
    socket: &mut WebSocket,
    pool: &SqlitePool,
    connection: &Connection,
    outgoing: &mut Outgoing<'_>,
    pending: &mut HashMap<String, Updates>,
    dispatch: Dispatch,
) -> Result<(), SocketFailure> {
    if dispatch.response.is_closed() {
        return Ok(());
    }
    if pending.len() >= MAX_PENDING {
        let _ = dispatch
            .response
            .send(AgentReply::failure("busy", "节点查询繁忙"));
        return Ok(());
    }
    check_current(pool, connection).await?;
    let id = dispatch.request.request_id.clone();
    outgoing
        .send(socket, ServerMessage::Task(dispatch.request))
        .await?;
    pending.insert(
        id,
        Updates {
            response: dispatch.response,
            progress: dispatch.progress,
            last_progress: None,
        },
    );
    Ok(())
}

async fn cancel_abandoned(
    socket: &mut WebSocket,
    outgoing: &mut Outgoing<'_>,
    pending: &mut HashMap<String, Updates>,
) -> Result<(), SocketFailure> {
    let abandoned: Vec<_> = pending
        .iter()
        .filter(|(_, result)| result.response.is_closed())
        .map(|(id, _)| id.clone())
        .collect();
    for request_id in abandoned {
        pending.remove(&request_id);
        outgoing
            .send(socket, ServerMessage::CancelTask { request_id })
            .await?;
    }
    Ok(())
}

fn valid_reply(reply: &AgentReply) -> bool {
    match reply {
        AgentReply::Success { .. } => true,
        AgentReply::Failure { code, message } => {
            !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && message.len() <= 512
                && !message.chars().any(char::is_control)
        }
    }
}

async fn check_current(pool: &SqlitePool, connection: &Connection) -> Result<(), SocketFailure> {
    let mut conn = pool.acquire().await.map_err(|_| SocketFailure::Internal)?;
    credentials::ensure_current(&mut conn, connection).await?;
    Ok(())
}

async fn store(
    pool: &SqlitePool,
    connection: &Connection,
    sample: &Value,
) -> Result<(), SocketFailure> {
    let mut tx = pool.begin().await.map_err(|_| SocketFailure::Internal)?;
    credentials::ensure_current(&mut tx, connection).await?;
    monitor::store_metrics(&mut tx, &connection.node_id, sample)
        .await
        .map_err(|_| SocketFailure::Internal)?;
    tx.commit().await.map_err(|_| SocketFailure::Internal)
}

struct Outgoing<'a> {
    node_id: &'a str,
    version: u16,
    sequence: u64,
}

impl Outgoing<'_> {
    async fn send(
        &mut self,
        socket: &mut WebSocket,
        message: ServerMessage,
    ) -> Result<(), SocketFailure> {
        let timestamp = unix_time().ok_or(SocketFailure::Internal)?;
        reply(
            socket,
            self.node_id,
            self.version,
            self.sequence,
            timestamp,
            message,
        )
        .await?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(SocketFailure::Internal)?;
        Ok(())
    }
}

struct Updates {
    response: oneshot::Sender<AgentReply>,
    progress: watch::Sender<Option<TaskProgress>>,
    last_progress: Option<Instant>,
}
