use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::auth::AuthError;
use crate::monitor::ApiError;
use crate::ssh_bus::BusError;

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
    "ssh_busy",
    "SSH 正在处理其他请求",
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
            "ssh_offline",
            "SSH 连接不可用",
        ),
        BusError::Busy => BUSY,
        BusError::Timeout => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "ssh_timeout",
            "等待 SSH 响应超时",
        ),
        BusError::Unsupported => ApiError::new(
            StatusCode::CONFLICT,
            "ssh_unavailable",
            "请检查 SSH 配置后操作",
        ),
        BusError::Rejected { .. } => ApiError::new(
            StatusCode::BAD_GATEWAY,
            "operation_unavailable",
            "节点暂不支持所请求的运维能力",
        ),
    }
}
