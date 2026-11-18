use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize, Serialize)]
struct ApiError {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    #[serde(flatten)]
    error: ApiError,
    request_id: String,
}

pub async fn response(request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;

    if is_api {
        let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let request_id = format!("{sequence:016x}");
        if response.status().is_client_error() || response.status().is_server_error() {
            response = normalize_error(response, request_id.clone()).await;
        }
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&request_id).expect("hex is a valid header value"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }

    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn normalize_error(response: Response, request_id: String) -> Response {
    let (mut parts, body) = response.into_parts();
    let error = to_bytes(body, 16 * 1024)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ApiError>(&bytes).ok())
        .unwrap_or_else(|| default_error(parts.status));

    if parts.status.is_server_error() {
        tracing::warn!(%request_id, status = %parts.status, code = %error.code, "API 请求失败");
    }

    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let body = Json(ErrorResponse { error, request_id })
        .into_response()
        .into_body();
    Response::from_parts(parts, body)
}

fn default_error(status: StatusCode) -> ApiError {
    let (code, message) = match status {
        StatusCode::UNAUTHORIZED => ("unauthorized", "请登录"),
        StatusCode::FORBIDDEN => ("forbidden", "请求未经授权"),
        StatusCode::NOT_FOUND => ("not_found", "资源不存在"),
        StatusCode::METHOD_NOT_ALLOWED => ("method_not_allowed", "请求方法不受支持"),
        StatusCode::PAYLOAD_TOO_LARGE => ("payload_too_large", "请求内容过大"),
        StatusCode::TOO_MANY_REQUESTS => ("rate_limited", "请求过于频繁"),
        status if status.is_client_error() => ("invalid_request", "请求内容无效"),
        _ => ("server_error", "中心暂时无法处理请求"),
    };
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}
