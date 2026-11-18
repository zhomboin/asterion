use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use asterion_protocol::{
    AgentMessage, ENROLLMENT_HEADER, Envelope, MAX_MESSAGE_SIZE, PROTOCOL_VERSION, ServerMessage,
    is_node_id,
};
use futures_util::{SinkExt, StreamExt};
use rustls::ClientConfig;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Error as WebSocketError;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue, header::AUTHORIZATION};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};

use crate::config::Config;
use crate::identity::{Identity, StateStore, read_enrollment};
use crate::{metadata, tls};

const RETRY_START: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Client {
    identity: Identity,
    store: StateStore,
    enrollment: Option<String>,
    tls: Arc<ClientConfig>,
    worker: crate::worker::Worker,
}

pub async fn run(config: Config) -> Result<()> {
    let tls = tls::client_config(config.ca_file.as_deref())?;
    let store = StateStore::open(&config.state_file)?;
    let endpoint = config.endpoint.as_str();
    let saved = store.load(endpoint)?;
    let enrollment = if saved.as_ref().is_none_or(|state| state.node_id.is_none())
        && let Some(path) = config.enrollment_file.as_deref()
    {
        read_enrollment(path)?
    } else {
        None
    };

    let identity = if let Some(saved) = saved {
        saved
    } else {
        ensure!(
            enrollment.is_some(),
            "首次启动需要准备 ASTERION_AGENT_ENROLLMENT_FILE"
        );
        let fresh = Identity::new(endpoint)?;
        store.save(&fresh)?;
        fresh
    };

    let mut client = Client {
        identity,
        store,
        enrollment,
        tls,
        worker: crate::worker::Worker::new()?,
    };
    let mut retry = RETRY_START;
    loop {
        let mut heartbeat_confirmed = false;
        let result = client.run_connection(&mut heartbeat_confirmed).await;
        if heartbeat_confirmed {
            retry = RETRY_START;
        }
        if let Err(error) = result {
            tracing::warn!(reason = %error, retry_seconds = retry.as_secs(), "Agent 连接中断");
        }
        sleep(retry).await;
        retry = (retry * 2).min(RETRY_MAX);
    }
}

impl Client {
    async fn connect(&self) -> Result<Socket> {
        let mut request = self
            .identity
            .endpoint
            .as_str()
            .into_client_request()
            .context("无法构建 Agent 连接请求")?;

        let bearer = format!("Bearer {}", self.identity.bearer);
        request
            .headers_mut()
            .insert(AUTHORIZATION, sensitive_header(&bearer)?);
        if let Some(token) = &self.enrollment {
            request.headers_mut().insert(
                HeaderName::from_static(ENROLLMENT_HEADER),
                sensitive_header(token)?,
            );
        }

        let config = WebSocketConfig::default()
            .write_buffer_size(0)
            .max_write_buffer_size(MAX_MESSAGE_SIZE * 2)
            .max_message_size(Some(MAX_MESSAGE_SIZE))
            .max_frame_size(Some(MAX_MESSAGE_SIZE));
        let connection = timeout(
            IO_TIMEOUT,
            connect_async_tls_with_config(
                request,
                Some(config),
                true,
                Some(Connector::Rustls(Arc::clone(&self.tls))),
            ),
        )
        .await
        .context("Agent TLS 连接超时")?;
        match connection {
            Ok((stream, _)) => Ok(stream),
            Err(WebSocketError::Http(response)) => {
                bail!("中心拒绝 Agent 连接（HTTP {}）", response.status().as_u16());
            }
            Err(WebSocketError::Io(error))
                if error
                    .get_ref()
                    .is_some_and(|source| source.is::<rustls::Error>()) =>
            {
                Err(error).context("中心 TLS 证书验证或协商失败")
            }
            Err(error) => Err(error).context("无法建立 Agent TLS 连接"),
        }
    }

    async fn run_connection(&mut self, heartbeat_confirmed: &mut bool) -> Result<()> {
        let mut socket = self.connect().await?;
        let reply = exchange(
            &mut socket,
            1,
            self.identity.node_id.clone(),
            AgentMessage::Hello(metadata::collect()),
            ServerMessage::Welcome {},
        )
        .await?;
        let node_id = reply.node_id.context("中心未返回节点标识")?;
        ensure!(
            self.identity
                .node_id
                .as_ref()
                .is_none_or(|saved| saved == &node_id),
            "中心返回的节点标识与已保存身份不匹配"
        );
        if self.identity.node_id.is_none() {
            self.identity.node_id = Some(node_id.clone());
            if let Err(error) = self.store.save(&self.identity) {
                self.identity.node_id = None;
                return Err(error);
            }
        }

        self.enrollment = None;
        tracing::info!(node_id, "Agent 已连接中心");
        crate::transport::run(&mut socket, node_id, &self.worker, heartbeat_confirmed).await
    }
}

async fn exchange(
    socket: &mut Socket,
    sequence: u64,
    node_id: Option<String>,
    message: AgentMessage,
    expected: ServerMessage,
) -> Result<Envelope<ServerMessage>> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Agent 系统时钟早于 Unix 纪元")?
        .as_secs()
        .try_into()
        .context("Agent 系统时钟超出协议范围")?;
    let outgoing = serde_json::to_string(&Envelope {
        protocol_version: PROTOCOL_VERSION,
        node_id,
        message_id: sequence,
        timestamp,
        message,
    })?;

    timeout(IO_TIMEOUT, async {
        socket
            .send(Message::Text(outgoing.into()))
            .await
            .context("无法发送 Agent 消息")?;
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let reply: Envelope<ServerMessage> = serde_json::from_str(&text)
                        .map_err(|_| anyhow!("中心返回了无效的 Agent 协议消息"))?;
                    ensure!(
                        reply.protocol_version == PROTOCOL_VERSION
                            && reply.message_id == sequence
                            && reply.node_id.as_deref().is_some_and(is_node_id)
                            && reply.message == expected,
                        "中心返回了不匹配的 Agent 协议确认"
                    );
                    return Ok(reply);
                }
                Some(Ok(Message::Ping(_))) => {
                    socket.flush().await.context("无法发送 WebSocket Pong")?;
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => bail!("中心已关闭 Agent 连接"),
                Some(Ok(_)) => bail!("中心返回了不支持的 WebSocket 消息"),
                Some(Err(error)) => return Err(error).context("无法读取中心的 Agent 消息"),
            }
        }
    })
    .await
    .context("等待中心确认超时")?
}

fn sensitive_header(value: &str) -> Result<HeaderValue> {
    let mut header = HeaderValue::from_str(value).context("Agent 凭证包含无效字符")?;
    header.set_sensitive(true);
    Ok(header)
}
