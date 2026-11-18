use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::agent_bus::BusError;
use crate::auth::AuthError;
use crate::monitor::ApiError;

pub(super) const NOT_FOUND: ApiError = ApiError::new(
    StatusCode::NOT_FOUND,
    "operation_not_found",
    "找不到运维任务",
);
pub(super) const INVALID_CONFIRMATION: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "invalid_confirmation",
    "运维确认已失效，请重新发起",
);
pub(super) const LIMIT_REACHED: ApiError = ApiError::new(
    StatusCode::CONFLICT,
    "operation_limit",
    "运维任务数量已达上限",
);
pub(super) const BUSY: ApiError = ApiError::new(
    StatusCode::SERVICE_UNAVAILABLE,
    "agent_busy",
    "Agent 正在处理其他请求",
);

#[derive(Debug)]
pub(super) enum ConfirmError {
    Operation(ApiError),
    Auth(AuthError),
}

impl From<ApiError> for ConfirmError {
    fn from(error: ApiError) -> Self {
        Self::Operation(error)
    }
}
impl From<AuthError> for ConfirmError {
    fn from(error: AuthError) -> Self {
        Self::Auth(error)
    }
}
impl IntoResponse for ConfirmError {
    fn into_response(self) -> Response {
        match self {
            Self::Operation(error) => error.into_response(),
            Self::Auth(error) => error.into_response(),
        }
    }
}

pub(super) fn bus_error(error: BusError) -> ApiError {
    match error {
        BusError::Offline => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_offline",
            "Agent 当前未连接",
        ),
        BusError::Busy => BUSY,
        BusError::Timeout => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "agent_timeout",
            "等待 Agent 响应超时",
        ),
        BusError::Unsupported => ApiError::new(
            StatusCode::CONFLICT,
            "agent_upgrade_required",
            "请升级 Agent 后操作",
        ),
        BusError::Rejected { .. } => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "operation_unavailable",
            "节点暂不支持所请求的运维能力",
        ),
    }
}
