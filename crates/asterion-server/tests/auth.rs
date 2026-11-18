use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use asterion_server::{auth, database};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

struct PasswordFile(PathBuf);

impl PasswordFile {
    fn new(password: &str) -> Self {
        let path = std::env::temp_dir().join(format!("asterion-auth-{:x}.secret", nonce()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).unwrap();
        writeln!(file, "{password}").unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PasswordFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn nonce() -> u64 {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

async fn configured_panel() -> (SqlitePool, String, String) {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let username = format!("operator-{:x}", nonce());
    let password = format!("local-check-{:x}{:x}", nonce(), nonce());
    let file = PasswordFile::new(&password);
    auth::bootstrap_admin(&pool, Some(&username), Some(file.path()))
        .await
        .unwrap();
    (pool, username, password)
}

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    data: Option<&Value>,
    cookie: Option<&str>,
    csrf: Option<&str>,
    fetch_site: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(csrf) = csrf {
        request = request.header("x-asterion-csrf", csrf);
    }
    if let Some(fetch_site) = fetch_site {
        request = request.header("sec-fetch-site", fetch_site);
    }
    let body = match data {
        Some(data) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(data.to_string())
        }
        None => Body::empty(),
    };
    router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn login(router: &Router, username: &str, password: &str) -> Response {
    let input = json!({ "username": username, "password": password });
    request(
        router,
        "POST",
        "/api/v1/auth/login",
        Some(&input),
        None,
        Some("1"),
        Some("same-origin"),
    )
    .await
}

#[tokio::test]
async fn rejects_unconfigured_bootstrap_and_keeps_existing_credentials() {
    let empty = database::connect("sqlite::memory:").await.unwrap();
    assert!(auth::bootstrap_admin(&empty, None, None).await.is_err());

    let (pool, username, password) = configured_panel().await;
    auth::bootstrap_admin(&pool, None, None).await.unwrap();
    assert!(
        auth::bootstrap_admin(&pool, Some(&format!("{username}-other")), None)
            .await
            .is_err()
    );
    let replacement = PasswordFile::new(&format!("{password}-replacement"));
    auth::bootstrap_admin(&pool, Some(&username), Some(replacement.path()))
        .await
        .unwrap();

    let hash = sqlx::query_scalar::<_, String>("SELECT password_hash FROM panel_admin")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(&password));
    let router = auth::routes().with_state(pool);
    assert_eq!(
        login(&router, &username, &password).await.status(),
        StatusCode::OK
    );
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_world_readable_or_symlinked_bootstrap_file() {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    let pool = database::connect("sqlite::memory:").await.unwrap();
    let username = format!("operator-{:x}", nonce());
    let file = PasswordFile::new(&format!("local-check-{:x}{:x}", nonce(), nonce()));
    fs::set_permissions(file.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        auth::bootstrap_admin(&pool, Some(&username), Some(file.path()))
            .await
            .is_err()
    );

    fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let link = PasswordFile(file.path().with_extension("link"));
    symlink(file.path(), link.path()).unwrap();
    assert!(
        auth::bootstrap_admin(&pool, Some(&username), Some(link.path()))
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM panel_admin")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn validates_csrf_and_invalid_credentials() {
    let (pool, username, password) = configured_panel().await;
    let router = auth::routes().with_state(pool);
    let input = json!({ "username": username, "password": password });
    for (csrf, fetch_site) in [
        (None, None),
        (Some("1"), Some("cross-site")),
        (Some("1"), Some("same-site")),
    ] {
        let response = request(
            &router,
            "POST",
            "/api/v1/auth/login",
            Some(&input),
            None,
            csrf,
            fetch_site,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_json(response).await["code"], "csrf_rejected");
    }

    let response = login(&router, &username, &format!("{password}-incorrect")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(response_json(response).await["code"], "invalid_credentials");
}

#[tokio::test]
async fn validates_cookie_expiry_and_logout() {
    let (pool, username, password) = configured_panel().await;
    let router = auth::routes().with_state(pool.clone());

    let response = request(&router, "GET", "/api/v1/me", None, None, None, None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = login(&router, &username, &password).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
    for attribute in [
        "Path=/",
        "Secure",
        "HttpOnly",
        "SameSite=Strict",
        "Max-Age=43200",
    ] {
        assert!(cookie.split("; ").any(|part| part == attribute));
    }
    let cookie = cookie.split(';').next().unwrap().to_owned();
    assert!(cookie.starts_with("__Host-asterion_session="));
    assert_eq!(response_json(response).await["username"], username);

    let response = request(
        &router,
        "GET",
        "/api/v1/me",
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["username"], username);

    sqlx::query("UPDATE panel_sessions SET created_at = 1, expires_at = 2")
        .execute(&pool)
        .await
        .unwrap();
    let response = request(
        &router,
        "GET",
        "/api/v1/me",
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = login(&router, &username, &password).await;
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let response = request(
        &router,
        "POST",
        "/api/v1/auth/logout",
        None,
        Some(&cookie),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let response = request(
        &router,
        "GET",
        "/api/v1/me",
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rate_limits_login_with_retry_after() {
    let pool = database::connect("sqlite::memory:").await.unwrap();
    let router = auth::routes().with_state(pool);

    for _ in 0..8 {
        assert_eq!(
            login(&router, "", "").await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let response = login(&router, "", "").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after));
    assert_eq!(response_json(response).await["code"], "login_rate_limited");
}
