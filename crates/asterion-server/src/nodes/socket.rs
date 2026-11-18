use std::sync::Arc;
use std::time::Duration;

use asterion_protocol::{
    AgentMessage, ENROLLMENT_HEADER, Envelope, Hello, MAX_MESSAGE_SIZE, MIN_PROTOCOL_VERSION,
    PROTOCOL_VERSION, ServerMessage, is_token,
};
use axum::Extension;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{Semaphore, watch};
use tokio::time::{Instant, timeout, timeout_at};

use super::credentials::{self, Candidate};
use super::error::ApiError;
use super::model::bounded_text;
use crate::agent_bus::AgentBus;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
static AGENT_CONNECTIONS: Semaphore = Semaphore::const_new(64);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SocketFailure {
    Protocol,
    Policy,
    Timeout,
    Disconnected,
    Internal,
}

impl SocketFailure {
    fn close(self) -> Option<CloseFrame> {
        let (code, reason) = match self {
            Self::Protocol => (close_code::PROTOCOL, "invalid protocol"),
            Self::Policy => (close_code::POLICY, "invalid agent message"),
            Self::Timeout => (close_code::POLICY, "agent timeout"),
            Self::Internal => (close_code::ERROR, "server error"),
            Self::Disconnected => return None,
        };
        Some(CloseFrame {
            code,
            reason: reason.into(),
        })
    }
}

impl From<ApiError> for SocketFailure {
    fn from(error: ApiError) -> Self {
        match error.status {
            StatusCode::UNAUTHORIZED => Self::Policy,
            _ => Self::Internal,
        }
    }
}

pub(super) async fn connect(
    State(pool): State<SqlitePool>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Extension(mut shutdown): Extension<watch::Receiver<bool>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let permit = AGENT_CONNECTIONS
        .try_acquire()
        .map_err(|_| ApiError::BUSY)?;
    let bearer = header_value(&headers, header::AUTHORIZATION.as_str())?
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, token)| scheme.eq_ignore_ascii_case("bearer") && is_token(token))
        .map(|(_, token)| token)
        .ok_or(ApiError::UNAUTHORIZED)?;
    let enrollment = header_value(&headers, ENROLLMENT_HEADER)?;
    if enrollment.is_some_and(|value| !is_token(value)) {
        return Err(ApiError::UNAUTHORIZED);
    }
    let candidate = credentials::authenticate(&pool, bearer, enrollment).await?;

    Ok(upgrade
        .max_frame_size(MAX_MESSAGE_SIZE)
        .max_message_size(MAX_MESSAGE_SIZE)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_MESSAGE_SIZE * 2)
        .on_upgrade(async move |mut socket| {
            let _permit = permit;
            let close = tokio::select! {
                biased;
                _ = shutdown.wait_for(|stopping| *stopping) => Some(CloseFrame {
                    code: close_code::AWAY,
                    reason: "server shutdown".into(),
                }),
                result = session(&mut socket, pool, candidate, bus) => result.err().and_then(SocketFailure::close),
            };
            if let Some(frame) = close {
                let _ = timeout(SEND_TIMEOUT, socket.send(Message::Close(Some(frame)))).await;
            }
        }))
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ApiError> {
    let values = headers.get_all(name);
    let mut entries = values.iter();
    let Some(value) = entries.next() else {
        return Ok(None);
    };
    if entries.next().is_some() {
        return Err(ApiError::UNAUTHORIZED);
    }
    value.to_str().map(Some).map_err(|_| ApiError::UNAUTHORIZED)
}

async fn session(
    socket: &mut WebSocket,
    pool: SqlitePool,
    candidate: Candidate,
    bus: Arc<AgentBus>,
) -> Result<(), SocketFailure> {
    let hello = receive(socket, Instant::now() + HANDSHAKE_TIMEOUT).await?;
    let metadata = validate_hello(&hello, &candidate.node_id)?;
    let (connection, seen_at) = credentials::connect(&pool, candidate, metadata).await?;
    reply(
        socket,
        &connection.node_id,
        hello.protocol_version,
        hello.message_id,
        seen_at,
        ServerMessage::Welcome {},
    )
    .await?;

    super::stream::run(socket, pool, connection, bus, hello.protocol_version).await
}

pub(super) async fn receive(
    socket: &mut WebSocket,
    deadline: Instant,
) -> Result<Envelope<AgentMessage>, SocketFailure> {
    timeout_at(deadline, async {
        for _ in 0..5 {
            match socket.recv().await {
                Some(Ok(Message::Text(text))) => return decode(text.as_str()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Err(_)) | Some(Ok(Message::Close(_))) | None => {
                    return Err(SocketFailure::Disconnected);
                }
                Some(Ok(Message::Binary(_))) => return Err(SocketFailure::Protocol),
            }
        }
        Err(SocketFailure::Policy)
    })
    .await
    .map_err(|_| SocketFailure::Timeout)?
}

pub(super) fn decode(text: &str) -> Result<Envelope<AgentMessage>, SocketFailure> {
    // Serde's flattened tagged message cannot also deny unknown envelope fields.
    const FIELDS: [&str; 6] = [
        "protocol_version",
        "node_id",
        "message_id",
        "timestamp",
        "type",
        "payload",
    ];
    let shape: Value = serde_json::from_str(text).map_err(|_| SocketFailure::Protocol)?;
    if !shape.as_object().is_some_and(|object| {
        object.len() == FIELDS.len() && FIELDS.iter().all(|key| object.contains_key(*key))
    }) {
        return Err(SocketFailure::Protocol);
    }
    serde_json::from_str(text).map_err(|_| SocketFailure::Protocol)
}

pub(super) fn validate_hello<'a>(
    envelope: &'a Envelope<AgentMessage>,
    node_id: &str,
) -> Result<&'a Hello, SocketFailure> {
    let AgentMessage::Hello(hello) = &envelope.message else {
        return Err(SocketFailure::Protocol);
    };
    version_check(envelope)?;
    if envelope.message_id != 1
        || envelope.node_id.as_ref().is_some_and(|id| id != node_id)
        || !bounded_text(&hello.agent_version, 64)
        || !bounded_text(&hello.os, 256)
        || !bounded_text(&hello.arch, 64)
        || hello
            .wsl_distro
            .as_deref()
            .is_some_and(|name| !bounded_text(name, 128))
    {
        return Err(SocketFailure::Policy);
    }
    Ok(hello)
}

pub(super) fn validate_heartbeat(
    envelope: &Envelope<AgentMessage>,
    node_id: &str,
    previous_message_id: u64,
) -> Result<(), SocketFailure> {
    version_check(envelope)?;
    if !matches!(envelope.message, AgentMessage::Heartbeat {}) {
        return Err(SocketFailure::Protocol);
    }
    if envelope.message_id <= previous_message_id || envelope.node_id.as_deref() != Some(node_id) {
        return Err(SocketFailure::Policy);
    }
    Ok(())
}

fn version_check(envelope: &Envelope<AgentMessage>) -> Result<(), SocketFailure> {
    if (MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&envelope.protocol_version)
        && envelope.timestamp >= 0
    {
        Ok(())
    } else {
        Err(SocketFailure::Protocol)
    }
}

pub(super) async fn reply(
    socket: &mut WebSocket,
    node_id: &str,
    protocol_version: u16,
    message_id: u64,
    timestamp: i64,
    message: ServerMessage,
) -> Result<(), SocketFailure> {
    let body = serde_json::to_string(&Envelope {
        protocol_version,
        node_id: Some(node_id.to_owned()),
        message_id,
        timestamp,
        message,
    })
    .map_err(|_| SocketFailure::Internal)?;
    timeout(SEND_TIMEOUT, socket.send(Message::Text(body.into())))
        .await
        .map_err(|_| SocketFailure::Timeout)?
        .map_err(|_| SocketFailure::Disconnected)
}
