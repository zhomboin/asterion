use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    pub const INVALID: Self = Self::new(StatusCode::BAD_REQUEST, "invalid_request", "参数无效");
    pub const NODE_NOT_FOUND: Self =
        Self::new(StatusCode::NOT_FOUND, "node_not_found", "找不到节点");

    pub const fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    pub fn invalid_json(error: axum::extract::rejection::JsonRejection) -> Self {
        Self::new(error.status(), "invalid_request", "请求内容无效")
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        let reason = if error.as_database_error().is_some() {
            "database"
        } else {
            "connection_or_decode"
        };
        tracing::warn!(reason, "监控或资产请求处理失败");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "中心暂时无法处理请求",
        )
    }
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}
