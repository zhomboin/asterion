use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use asterion_protocol::{
    AgentMessage, AgentReply, Envelope, HEARTBEAT_INTERVAL_SECS, MAX_MESSAGE_SIZE,
    MAX_SAMPLE_INTERVAL_SECS, PROTOCOL_VERSION, ServerMessage, is_node_id, unix_time,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};
use tokio_tungstenite::tungstenite::protocol::Message;

use crate::client::Socket;
use crate::worker::{Completed, Worker};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(HEARTBEAT_INTERVAL_SECS);
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TASK_TTL: i64 = 120;

pub async fn run(
    socket: &mut Socket,
    node_id: String,
    worker: &Worker,
    heartbeat_confirmed: &mut bool,
) -> Result<()> {
    let mut sender = Sender {
        node_id: &node_id,
        sequence: 1,
    };
    let mut heartbeat = interval_at(Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending_heartbeat = None::<(u64, Instant)>;
    let mut sample_interval = None::<Duration>;
    let mut next_sample = Instant::now();
    let mut sample_pending = false;
    let mut jobs = JoinSet::new();
    let (progress, mut updates) = mpsc::channel(8);
    let mut tasks = HashMap::<String, AbortHandle>::new();
    let mut last_control_id = 0;
    let mut terminals = crate::terminal_proxy::TerminalProxy::new();
    let mut container_terminal = crate::terminal_proxy::TerminalProxy::container();
    let mut container_session = None::<String>;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if let Some((_, sent)) = pending_heartbeat {
                    ensure!(sent.elapsed() < ACK_TIMEOUT, "等待中心心跳确认超时");
                } else {
                    let id = sender.send(socket, AgentMessage::Heartbeat {}).await?;
                    pending_heartbeat = Some((id, Instant::now()));
                }
            }
            _ = sleep_until(next_sample), if sample_interval.is_some() => {
                if let Some(period) = sample_interval {
                    next_sample = Instant::now() + period;
                }
                if !sample_pending {
                    sample_pending = worker.sample(&mut jobs);
                }
            }
            incoming = socket.next() => {
                let Some(incoming) = incoming else { bail!("中心已关闭连接"); };
                let frame = incoming.context("无法读取中心消息")?;
                let message: Envelope<ServerMessage> = match frame {
                    Message::Text(text) => serde_json::from_str(&text)
                        .map_err(|_| anyhow::anyhow!("中心返回无效协议消息"))?,
                    Message::Ping(_) => {
                        timeout(SEND_TIMEOUT, socket.flush()).await??;
                        continue;
                    }
                    Message::Pong(_) => continue,
                    Message::Close(_) => bail!("中心已关闭连接"),
                    _ => bail!("中心返回不支持的消息"),
                };
                ensure!(message.protocol_version == PROTOCOL_VERSION
                    && message.node_id.as_deref() == Some(node_id.as_str())
                    && message.timestamp >= 0, "中心消息与当前节点或协议不匹配");
                if matches!(message.message, ServerMessage::HeartbeatAck {}) {
                    ensure!(pending_heartbeat.is_some_and(|(id, _)| id == message.message_id),
                            "中心心跳确认序号不匹配");
                    pending_heartbeat = None;
                    *heartbeat_confirmed = true;
                    continue;
                }

                ensure!(message.message_id > last_control_id, "中心控制消息序号无效");
                last_control_id = message.message_id;
                match message.message {
                    ServerMessage::Configure { sample_interval_secs } => {
                        ensure!(sample_interval.is_none() && (5..=MAX_SAMPLE_INTERVAL_SECS).contains(&sample_interval_secs),
                            "中心采样间隔无效");
                        sample_interval = Some(Duration::from_secs(sample_interval_secs));
                        next_sample = Instant::now();
                    }
                    ServerMessage::Task(request) => {
                        ensure!(is_node_id(&request.request_id) && request.kind.len() <= 64
                            && !tasks.contains_key(&request.request_id), "中心查询标识无效");
                        let id = request.request_id.clone();
                        let remaining = unix_time().map_or(0, |now| request.expires_at.saturating_sub(now));
                        let limit = if request.kind == "operation_execute" { 930 } else { MAX_TASK_TTL };
                        if remaining <= 0 || remaining > limit {
                            sender.answer(socket, id, AgentReply::failure("timeout", "查询截止时间无效")).await?;
                        } else if request.kind.starts_with("terminal_") {
                            let open = request.kind == "terminal_container_open";
                            let session = request.args["id"].as_str().map(str::to_owned);
                            let reply = if open || request.kind == "terminal_container_capability" ||
                                (session.is_some() && container_session == session && request.kind != "terminal_open") {
                                container_terminal.handle(request).await
                            } else {
                                terminals.handle(request).await
                            };
                            if open && matches!(&reply, AgentReply::Success { value } if value["opened"] == true) {
                                container_session = session;
                            }
                            sender.answer(socket, id, reply).await?;
                        } else if let Some(task) = worker.task(&mut jobs, request, &progress) {
                            tasks.insert(id, task);
                        } else {
                            sender.answer(socket, id, AgentReply::failure("busy", "节点查询繁忙")).await?;
                        }
                    }
                    ServerMessage::CancelTask { request_id } => {
                        if let Some(task) = tasks.remove(&request_id) {
                            task.abort();
                            sender.answer(socket, request_id, AgentReply::failure("cancelled", "Agent 已取消任务")).await?;
                        }
                    }
                    _ => bail!("中心发送了意外协议消息"),
                }
            }
            update = updates.recv() => {
                if let Some((request_id, progress)) = update
                    && tasks.contains_key(&request_id)
                {
                    sender.send(socket, AgentMessage::TaskProgress { request_id, progress }).await?;
                }
            }
            result = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Some(Ok(Completed::Metrics(sample))) => {
                        sample_pending = false;
                        if let Some(sample) = sample {
                            sender.send(socket, AgentMessage::Metrics { sample }).await?;
                        }
                    }
                    Some(Ok(Completed::Task { id, reply })) => {
                        if tasks.remove(&id).is_some() {
                            sender.answer(socket, id, reply).await?;
                        }
                    }
                    Some(Err(err)) if !err.is_cancelled() => bail!("采集任务异常退出"),
                    _ => {}
                }
            }
        }
    }
}

struct Sender<'a> {
    node_id: &'a str,
    sequence: u64,
}

impl Sender<'_> {
    async fn serialize(
        &mut self,
        socket: &mut Socket,
        mut envelope: Envelope<AgentMessage>,
    ) -> Result<u64> {
        let mut text = serde_json::to_string(&envelope)?;
        if text.len() > MAX_MESSAGE_SIZE {
            if let AgentMessage::TaskReply { ref mut reply, .. } = envelope.message {
                *reply = AgentReply::failure("output_limit", "采集结果超过大小限制");
                text = serde_json::to_string(&envelope)?;
            } else {
                return Ok(envelope.message_id);
            }
        }
        timeout(SEND_TIMEOUT, socket.send(Message::Text(text.into()))).await??;
        Ok(envelope.message_id)
    }

    async fn send(&mut self, socket: &mut Socket, message: AgentMessage) -> Result<u64> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("Agent 消息序号已达上限")?;
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            node_id: Some(self.node_id.to_owned()),
            message_id: self.sequence,
            timestamp: unix_time().context("Agent 时钟无效")?,
            message,
        };
        self.serialize(socket, envelope).await
    }

    async fn answer(
        &mut self,
        socket: &mut Socket,
        request_id: String,
        reply: AgentReply,
    ) -> Result<()> {
        self.send(socket, AgentMessage::TaskReply { request_id, reply })
            .await?;
        Ok(())
    }
}
