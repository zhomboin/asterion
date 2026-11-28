use std::net::SocketAddr;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn};
use axum::response::IntoResponse;

/// 只有通过本机地址和来源校验的请求可以使用本地 SSH 身份。
#[derive(Clone, Copy)]
pub(crate) struct LocalPanel;

/// 只供桌面进程内部调用，此入口不绑定网络地址。
#[cfg(not(feature = "openssh"))]
pub fn in_process(router: Router) -> Router {
    router.layer(axum::Extension(LocalPanel))
}

pub fn router(router: Router, address: SocketAddr) -> Router {
    let authority = address.to_string();
    let origin = format!("http://{authority}");
    router.layer(from_fn(move |mut request: Request, next: Next| {
        let valid = request
            .headers()
            .get(header::HOST)
            .is_some_and(|value| value == authority.as_str())
            && request
                .headers()
                .get_all(header::ORIGIN)
                .iter()
                .all(|value| value == origin.as_str());
        async move {
            if !valid {
                return StatusCode::FORBIDDEN.into_response();
            }
            request.extensions_mut().insert(LocalPanel);
            next.run(request).await
        }
    }))
}
