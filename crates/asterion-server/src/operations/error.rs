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

pub(super) fn file_error(error: BusError) -> ApiError {
    let BusError::Rejected { ref code, .. } = error else {
        return bus_error(error);
    };
    let message = match code.as_str() {
        "nginx_missing" => "未找到可识别的 Nginx 主进程，可按绝对路径打开文件",
        "nginx_multiple" => "发现多个 Nginx 主进程，无法确定重载目标，可按绝对路径打开文件",
        "nginx_arguments" => "无法还原 Nginx 启动参数，可按绝对路径打开文件",
        "nginx_check" => "nginx -T 执行失败，请检查现有配置和 SSH 账号权限",
        "nginx_dump" | "target_changed" => "配置清单或文件已变化，请重新读取",
        "nginx_too_large" => "Nginx 配置清单超过读取限制",
        "file_too_large" => "编辑器支持最大 64 KiB 的文件",
        "file_not_text" => "请选择 UTF-8 编码的普通文本文件",
        "not_found" => "找不到文件或链接目标",
        "permission_denied" | "operation_denied" => "SSH 账号或节点策略不允许此操作",
        "unavailable" => "文件编辑需要目标机提供 Python 3.9 或以上版本",
        _ => return bus_error(error),
    };
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "file_unavailable",
        message,
    )
}
