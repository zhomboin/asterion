use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug)]
pub(super) struct ApiError {
    pub status: StatusCode,
    code: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    pub(super) fn deployment(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            code: "ssh_deployment",
            message,
        }
    }
    pub const INVALID_INPUT: Self = Self {
        status: StatusCode::BAD_REQUEST,
        code: "invalid_node_input",
        message: "节点参数无效",
    };
    pub const NOT_FOUND: Self = Self {
        status: StatusCode::NOT_FOUND,
        code: "node_not_found",
        message: "找不到节点",
    };
    pub const IN_USE: Self = Self {
        status: StatusCode::CONFLICT,
        code: "node_in_use",
        message: "节点仍有关联的服务资产",
    };
    pub const UNAUTHORIZED: Self = Self {
        status: StatusCode::UNAUTHORIZED,
        code: "invalid_agent_credentials",
        message: "Agent 凭证无效或已失效",
    };
    pub const BUSY: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "agent_connection_limit",
        message: "中心连接繁忙，请稍后重试",
    };
    pub const INTERNAL: Self = Self {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "internal_error",
        message: "无法处理节点请求",
    };

    pub fn invalid_json(error: axum::extract::rejection::JsonRejection) -> Self {
        Self {
            status: error.status(),
            code: "invalid_node_input",
            message: "节点参数无效",
        }
    }

    pub fn internal(reason: &'static str) -> Self {
        tracing::error!(reason, "节点请求处理失败");
        Self::INTERNAL
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        let kind = if error.as_database_error().is_some() {
            "database"
        } else {
            "connection_or_decode"
        };
        Self::internal(kind)
    }
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
