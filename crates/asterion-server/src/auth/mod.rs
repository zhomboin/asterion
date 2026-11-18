mod credentials;
mod rate;
mod session;

use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};

use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::agent_bus::AgentBus;
use crate::terminal::TerminalHub;

use self::rate::LoginThrottle;
use self::session::{expire_cookie, login_cookie, session_hash};

pub use self::credentials::bootstrap_admin;
pub use self::session::AdminSession;

pub fn routes() -> Router<SqlitePool> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/me", get(me))
        .layer(DefaultBodyLimit::max(2048))
        .layer(Extension(Arc::new(LoginThrottle::new())))
}

pub struct Csrf;

impl<S: Send + Sync> FromRequestParts<S> for Csrf {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let mut csrf_headers = parts.headers.get_all("x-asterion-csrf").iter();
        let permitted_site = parts
            .headers
            .get_all("sec-fetch-site")
            .iter()
            .all(|value| value == "same-origin" || value == "none");
        if csrf_headers.next().is_some_and(|value| value == "1")
            && csrf_headers.next().is_none()
            && permitted_site
        {
            return Ok(Self);
        }
        Err(AuthError::Csrf)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginInput {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct UserResponse {
    username: String,
}

async fn login(
    _: Csrf,
    State(pool): State<SqlitePool>,
    Extension(throttle): Extension<Arc<LoginThrottle>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    input: Result<Json<LoginInput>, JsonRejection>,
) -> Result<Response, AuthError> {
    let Json(input) = input.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AuthError::TooLarge
        } else {
            AuthError::InvalidRequest
        }
    })?;
    let permit = throttle.acquire(peer.map(|Extension(ConnectInfo(address))| address.ip()))?;
    if !credentials::valid_username(&input.username)
        || !credentials::valid_password(&input.password)
    {
        return Err(AuthError::InvalidCredentials);
    }

    let (username, password_hash) = sqlx::query_as::<_, (String, String)>(
        "SELECT username, password_hash FROM panel_admin WHERE id = 1",
    )
    .fetch_optional(&pool)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::InvalidCredentials)?;
    let username = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let password_matches = credentials::verify_password(&input.password, &password_hash)?;
        Ok::<_, AuthError>((password_matches && input.username == username).then_some(username))
    })
    .await
    .map_err(AuthError::internal)??
    .ok_or(AuthError::InvalidCredentials)?;

    let cookie = login_cookie(&pool).await?;
    tracing::info!(action = "panel_login", outcome = "success", "管理面板登录");
    Ok((
        [
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::SET_COOKIE, cookie),
        ],
        Json(UserResponse { username }),
    )
        .into_response())
}

async fn logout(
    _: Csrf,
    State(pool): State<SqlitePool>,
    terminals: Option<Extension<Arc<TerminalHub>>>,
    bus: Option<Extension<Arc<AgentBus>>>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    if let Some(hash) = session_hash(&headers) {
        let deletion = sqlx::query("DELETE FROM panel_sessions WHERE token_hash = ?")
            .bind(hash.as_slice())
            .execute(&pool)
            .await
            .map_err(AuthError::internal)?;
        if let (Some(Extension(terminals)), Some(Extension(bus))) = (terminals, bus) {
            terminals.revoke_binding(&hash, &bus);
        }
        if deletion.rows_affected() != 0 {
            tracing::info!(action = "panel_logout", outcome = "success", "管理面板注销");
        }
    }

    Ok((
        StatusCode::NO_CONTENT,
        [
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::SET_COOKIE, expire_cookie()),
        ],
    )
        .into_response())
}

async fn me(session: AdminSession) -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(UserResponse {
            username: session.username,
        }),
    )
}

#[derive(Debug)]
pub enum AuthError {
    Csrf,
    InvalidRequest,
    TooLarge,
    InvalidCredentials,
    Unauthenticated,
    RateLimited(u64),
    Internal,
}

impl AuthError {
    fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "管理面板鉴权失败");
        Self::Internal
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Csrf => (StatusCode::FORBIDDEN, "csrf_rejected", "请求来源校验失败"),
            Self::InvalidRequest => (StatusCode::BAD_REQUEST, "invalid_request", "请求内容无效"),
            Self::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "请求内容超出限制",
            ),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "用户名或口令错误",
            ),
            Self::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated", "请登录"),
            Self::RateLimited(_) => (
                StatusCode::TOO_MANY_REQUESTS,
                "login_rate_limited",
                "尝试过多，请稍后重试",
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "管理面板暂不可用",
            ),
        };
        let mut response = (
            status,
            [(header::CACHE_CONTROL, "no-store")],
            Json(ErrorResponse { code, message }),
        )
            .into_response();
        if let Self::RateLimited(retry_after) = self {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
        }
        response
    }
}

pub async fn verify_step_up(
    pool: &SqlitePool,
    session: &AdminSession,
    password: &str,
) -> Result<(), AuthError> {
    static THROTTLE: LazyLock<LoginThrottle> = LazyLock::new(LoginThrottle::new);
    let permit = THROTTLE.acquire(None)?;
    if !credentials::valid_password(password) {
        return Err(AuthError::InvalidCredentials);
    }
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM panel_admin WHERE id = 1 AND username = ?")
            .bind(&session.username)
            .fetch_optional(pool)
            .await
            .map_err(AuthError::internal)?
            .ok_or(AuthError::Unauthenticated)?;
    let password = password.to_owned();
    let matches = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        credentials::verify_password(&password, &hash)
    })
    .await
    .map_err(AuthError::internal)??;
    if matches {
        Ok(())
    } else {
        Err(AuthError::InvalidCredentials)
    }
}
