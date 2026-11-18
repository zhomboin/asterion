use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use asterion_protocol::is_node_id;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::agent_bus::{AgentBus, BusError};
use crate::auth::{AdminSession, AuthError, Csrf, verify_step_up};
use crate::monitor::ApiError;

const LIFETIME: Duration = Duration::from_secs(900);
const OPEN_TIMEOUT: Duration = Duration::from_secs(12);
const MISSING: ApiError = ApiError::new(StatusCode::NOT_FOUND, "terminal_closed", "终端会话已结束");
const DISABLED: ApiError =
    ApiError::new(StatusCode::FORBIDDEN, "terminal_disabled", "中心未启用终端");

pub fn routes(enabled: bool) -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/nodes/{node}/terminal/capability", get(capability))
        .route("/api/v1/nodes/{node}/terminal", post(open))
        .route("/api/v1/terminal/{id}", get(poll).delete(close))
        .route("/api/v1/terminal/{id}/input", post(input))
        .route("/api/v1/terminal/{id}/resize", post(resize))
        .layer(Extension(Arc::new(TerminalHub {
            enabled,
            sessions: Mutex::new(HashMap::new()),
        })))
}

struct Session {
    node: String,
    binding: [u8; 32],
    deadline: Instant,
}

struct TerminalHub {
    enabled: bool,
    sessions: Mutex<HashMap<String, Session>>,
}

impl TerminalHub {
    fn reserve(&self, node: &str, session: &AdminSession) -> Result<String, ApiError> {
        if !self.enabled {
            return Err(DISABLED);
        }
        let mut sessions = self.sessions.lock().unwrap_or_else(|err| err.into_inner());
        let now = Instant::now();
        sessions.retain(|_, session| session.deadline > now);
        if sessions.len() >= 16 {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "terminal_limit",
                "终端会话数已达上限",
            ));
        }
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "中心暂时无法创建会话",
            )
        })?;
        let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        sessions.insert(
            id.clone(),
            Session {
                node: node.to_owned(),
                binding: *session.binding(),
                deadline: now + LIFETIME,
            },
        );
        Ok(id)
    }

    fn node(&self, id: &str, session: &AdminSession) -> Result<String, ApiError> {
        if !self.enabled || !is_node_id(id) {
            return Err(MISSING);
        }
        self.sessions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(id)
            .filter(|terminal| {
                terminal.deadline > Instant::now() && &terminal.binding == session.binding()
            })
            .map(|terminal| terminal.node.clone())
            .ok_or(MISSING)
    }

    fn forget(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(id)
            .is_some()
    }
}

struct OpeningReservation<'a> {
    hub: &'a TerminalHub,
    bus: &'a Arc<AgentBus>,
    node: &'a str,
    id: Option<String>,
    dispatched: bool,
}

impl OpeningReservation<'_> {
    fn commit(mut self) -> String {
        self.id.take().expect("a reservation has a terminal ID")
    }
}

impl Drop for OpeningReservation<'_> {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        self.hub.forget(&id);
        if self.dispatched {
            let node = self.node.to_owned();
            let bus = Arc::clone(self.bus);
            // 结果未确认时，尝试关闭可能已启动的同一会话。
            tokio::spawn(async move {
                let _ = bus.query(&node, "terminal_close", json!({"id":id})).await;
            });
        }
    }
}

async fn capability(
    _: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(node): Path<String>,
) -> Result<Json<Value>, ApiError> {
    crate::monitor::ensure_node(&pool, &node).await?;
    if !hub.enabled {
        return Ok(Json(json!({"enabled":false})));
    }
    match bus.query(&node, "terminal_capability", json!({})).await {
        Ok(result) => Ok(Json(json!({"enabled": result["enabled"] == true}))),
        Err(BusError::Unsupported | BusError::Offline) => Ok(Json(json!({"enabled":false}))),
        Err(error) => Err(bus_error(error)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dimensions {
    cols: u16,
    rows: u16,
}

impl Dimensions {
    fn valid(&self) -> bool {
        (20..=400).contains(&self.cols) && (5..=200).contains(&self.rows)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opening {
    password: String,
    cols: u16,
    rows: u16,
}

async fn open(
    _: Csrf,
    session: AdminSession,
    State(pool): State<SqlitePool>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Extension(bus): Extension<Arc<AgentBus>>,
    Path(node): Path<String>,
    Json(body): Json<Opening>,
) -> Result<(StatusCode, Json<Value>), OpenError> {
    if !hub.enabled {
        return Err(OpenError::Api(DISABLED));
    }
    if !(Dimensions {
        cols: body.cols,
        rows: body.rows,
    })
    .valid()
    {
        return Err(OpenError::Api(ApiError::INVALID));
    }
    crate::monitor::ensure_node(&pool, &node)
        .await
        .map_err(OpenError::Api)?;
    verify_step_up(&pool, &session, &body.password)
        .await
        .map_err(|error| match error {
            AuthError::InvalidCredentials => OpenError::Api(ApiError::new(
                StatusCode::FORBIDDEN,
                "invalid_step_up_password",
                "管理员口令验证失败",
            )),
            other => OpenError::Auth(other),
        })?;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|error| OpenError::Api(error.into()))?;
    session
        .ensure_active(&mut conn)
        .await
        .map_err(OpenError::Auth)?;
    let id = hub.reserve(&node, &session).map_err(OpenError::Api)?;
    let mut reservation = OpeningReservation {
        hub: &hub,
        bus: &bus,
        node: &node,
        id: Some(id.clone()),
        dispatched: false,
    };
    let pending = bus
        .enqueue(
            &node,
            &id,
            "terminal_open",
            json!({"id":id,"cols":body.cols,"rows":body.rows}),
            OPEN_TIMEOUT,
        )
        .map_err(|error| OpenError::Api(bus_error(error)))?;
    reservation.dispatched = true;
    drop(conn);
    let result = pending
        .wait()
        .await
        .map_err(|error| OpenError::Api(bus_error(error)))?;
    if result["opened"] != true {
        return Err(OpenError::Api(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "agent_invalid_reply",
            "节点未确认终端开启",
        )));
    }
    let id = reservation.commit();
    tracing::info!(action = "terminal_open", actor = %session.username, node_id = %node, terminal_id = %id, "终端会话已开启");
    Ok((StatusCode::CREATED, Json(json!({"id":id}))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Offset {
    #[serde(default)]
    offset: u64,
}

async fn poll(
    session: AdminSession,
    Extension(bus): Extension<Arc<AgentBus>>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Path(id): Path<String>,
    Query(query): Query<Offset>,
) -> Result<Json<Value>, ApiError> {
    query_session(
        &hub,
        &bus,
        &id,
        &session,
        "terminal_poll",
        json!({"id":id,"offset":query.offset}),
    )
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    data: String,
}

async fn input(
    _: Csrf,
    session: AdminSession,
    Extension(bus): Extension<Arc<AgentBus>>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Path(id): Path<String>,
    Json(input): Json<Input>,
) -> Result<StatusCode, ApiError> {
    if input.data.is_empty() || input.data.len() > 5464 {
        return Err(ApiError::INVALID);
    }
    query_session(
        &hub,
        &bus,
        &id,
        &session,
        "terminal_input",
        json!({"id":id,"data":input.data}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resize(
    _: Csrf,
    session: AdminSession,
    Extension(bus): Extension<Arc<AgentBus>>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Path(id): Path<String>,
    Json(size): Json<Dimensions>,
) -> Result<StatusCode, ApiError> {
    if !size.valid() {
        return Err(ApiError::INVALID);
    }
    query_session(
        &hub,
        &bus,
        &id,
        &session,
        "terminal_resize",
        json!({"id":id,"cols":size.cols,"rows":size.rows}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn query_session(
    hub: &TerminalHub,
    bus: &AgentBus,
    id: &str,
    session: &AdminSession,
    kind: &str,
    args: Value,
) -> Result<Value, ApiError> {
    let node = hub.node(id, session)?;
    let result = bus.query(&node, kind, args).await;
    let ended = match &result {
        Ok(reply) => reply["closed"] == true,
        Err(error) => session_unavailable(error),
    };
    if ended && hub.forget(id) {
        tracing::info!(action = "terminal_close", actor = %session.username, node_id = %node, terminal_id = %id, "终端会话已结束");
    }
    result.map_err(bus_error)
}

fn session_unavailable(error: &BusError) -> bool {
    matches!(error, BusError::Offline | BusError::Unsupported)
        || matches!(error, BusError::Rejected { code, .. } if matches!(code.as_str(), "terminal_closed" | "terminal_disabled"))
}

async fn close(
    _: Csrf,
    session: AdminSession,
    Extension(bus): Extension<Arc<AgentBus>>,
    Extension(hub): Extension<Arc<TerminalHub>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let node = hub.node(&id, &session)?;
    match bus.query(&node, "terminal_close", json!({"id":id})).await {
        Ok(_) => {}
        Err(error) if session_unavailable(&error) => {}
        Err(error) => return Err(bus_error(error)),
    }
    hub.forget(&id);
    tracing::info!(action = "terminal_close", actor = %session.username, node_id = %node, terminal_id = %id, "终端会话已关闭");
    Ok(StatusCode::NO_CONTENT)
}

fn bus_error(error: BusError) -> ApiError {
    let (status, code, message) = match error {
        BusError::Offline => (
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_offline",
            "节点 Agent 未连接",
        ),
        BusError::Unsupported => (
            StatusCode::CONFLICT,
            "agent_upgrade_required",
            "请升级节点 Agent",
        ),
        BusError::Busy => (
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_busy",
            "节点任务繁忙",
        ),
        BusError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "agent_timeout", "节点响应超时"),
        BusError::Rejected { code, .. } => match code.as_str() {
            "terminal_closed" => return MISSING,
            "terminal_disabled" => (StatusCode::FORBIDDEN, "terminal_disabled", "节点未启用终端"),
            "permission_denied" => (
                StatusCode::FORBIDDEN,
                "permission_denied",
                "节点访问权限不足",
            ),
            "not_found" => (StatusCode::NOT_FOUND, "not_found", "节点未找到记录"),
            "invalid_request" | "invalid_input" => return ApiError::INVALID,
            _ => (
                StatusCode::BAD_GATEWAY,
                "agent_rejected",
                "节点拒绝了此请求",
            ),
        },
    };
    ApiError::new(status, code, message)
}

enum OpenError {
    Api(ApiError),
    Auth(AuthError),
}

impl IntoResponse for OpenError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Auth(error) => error.into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use asterion_protocol::AgentReply;

    use super::*;
    use crate::agent_bus::Control;

    #[tokio::test]
    async fn abandoned_terminal_open_releases_its_slot_and_closes_only_its_id() {
        let id = "1".repeat(32);
        let neighbor = "2".repeat(32);
        let node = "3".repeat(32);
        let now = Instant::now();
        let session = || Session {
            node: node.clone(),
            binding: [0; 32],
            deadline: now + LIFETIME,
        };
        let hub = TerminalHub {
            enabled: true,
            sessions: Mutex::new(HashMap::from([
                (id.clone(), session()),
                (neighbor.clone(), session()),
            ])),
        };
        let bus = Arc::new(AgentBus::new(15));
        let mut agent = bus.attach(&node, true);
        let reservation = OpeningReservation {
            hub: &hub,
            bus: &bus,
            node: &node,
            id: Some(id.clone()),
            dispatched: true,
        };
        drop(reservation);

        {
            let sessions = hub.sessions.lock().unwrap();
            assert!(!sessions.contains_key(&id));
            assert!(sessions.contains_key(&neighbor));
        }
        let Control::Task(dispatch) = tokio::time::timeout(Duration::from_secs(2), agent.next())
            .await
            .unwrap()
        else {
            panic!("expected cleanup dispatch");
        };
        assert_eq!(dispatch.request.kind, "terminal_close");
        assert_eq!(dispatch.request.args, json!({"id":id}));
        dispatch
            .response
            .send(AgentReply::Success { value: Value::Null })
            .unwrap();
    }
}
