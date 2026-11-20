use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, header};
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};

use super::AuthError;

const COOKIE: &str = "__Host-asterion_session";
const ATTRIBUTES: &str = "Path=/; Secure; HttpOnly; SameSite=Strict";
pub(super) const LOCAL_SESSION: &str = "x-asterion-local-session";
const SESSION_LIFETIME: i64 = 12 * 60 * 60;
const MAX_SESSIONS: i64 = 16;
const TOKEN_BYTES: usize = 32;

pub struct AdminSession {
    pub username: String,
    binding: [u8; 32],
}

impl AdminSession {
    pub fn binding(&self) -> &[u8; 32] {
        &self.binding
    }

    pub(crate) async fn ensure_active(&self, conn: &mut SqliteConnection) -> Result<(), AuthError> {
        let active = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM panel_sessions s JOIN panel_admin a ON a.id = s.admin_id
             WHERE s.token_hash = ? AND s.expires_at > unixepoch() AND a.username = ?)",
        )
        .bind(self.binding.as_slice())
        .bind(&self.username)
        .fetch_one(conn)
        .await
        .map_err(AuthError::internal)?;
        if active {
            Ok(())
        } else {
            Err(AuthError::Unauthenticated)
        }
    }
}

impl FromRequestParts<SqlitePool> for AdminSession {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        pool: &SqlitePool,
    ) -> Result<Self, Self::Rejection> {
        let hash = session_hash(
            &parts.headers,
            parts.extensions.get::<crate::local::LocalPanel>().is_some(),
        )
        .ok_or(AuthError::Unauthenticated)?;
        let username = sqlx::query_scalar(
            "SELECT panel_admin.username FROM panel_sessions
             INNER JOIN panel_admin ON panel_admin.id = panel_sessions.admin_id
             WHERE panel_sessions.token_hash = ? AND panel_sessions.expires_at > unixepoch()",
        )
        .bind(hash.as_slice())
        .fetch_optional(pool)
        .await
        .map_err(AuthError::internal)?
        .ok_or(AuthError::Unauthenticated)?;
        Ok(Self {
            username,
            binding: hash,
        })
    }
}

pub(super) fn session_hash(headers: &HeaderMap, local: bool) -> Option<[u8; 32]> {
    if local {
        let mut values = headers.get_all(LOCAL_SESSION).iter();
        let value = values.next()?.to_str().ok()?;
        return (values.next().is_none() && valid_token(value)).then(|| sha256(value));
    }
    let mut token = None;
    for header in headers.get_all(header::COOKIE) {
        let header = header.to_str().ok()?;
        for part in header.split(';') {
            let Some((key, value)) = part.trim().split_once('=') else {
                continue;
            };
            if key != COOKIE {
                continue;
            }
            if token.is_some() || !valid_token(value) {
                return None;
            }
            token = Some(value);
        }
    }
    token.map(sha256)
}

fn valid_token(value: &str) -> bool {
    value.len() == TOKEN_BYTES * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

pub(super) async fn issue_session(
    pool: &SqlitePool,
    local: bool,
) -> Result<HeaderValue, AuthError> {
    let token = random_token()?;
    let hash = sha256(&token);
    let value = if local {
        token
    } else {
        format!("{COOKIE}={token}; Max-Age={SESSION_LIFETIME}; {ATTRIBUTES}")
    };
    let header = HeaderValue::from_str(&value).map_err(AuthError::internal)?;

    let mut transaction = pool.begin().await.map_err(AuthError::internal)?;
    sqlx::query(
        "INSERT INTO panel_sessions (token_hash, admin_id, expires_at) VALUES (?, 1, unixepoch() + ?)",
    )
    .bind(hash.as_slice())
    .bind(SESSION_LIFETIME)
    .execute(&mut *transaction)
    .await
    .map_err(AuthError::internal)?;
    sqlx::query(
        "DELETE FROM panel_sessions WHERE expires_at <= unixepoch() OR rowid IN (
            SELECT rowid FROM panel_sessions ORDER BY created_at DESC, rowid DESC LIMIT -1 OFFSET ?
        )",
    )
    .bind(MAX_SESSIONS)
    .execute(&mut *transaction)
    .await
    .map_err(AuthError::internal)?;
    transaction.commit().await.map_err(AuthError::internal)?;
    Ok(header)
}

pub(super) fn expire_cookie() -> HeaderValue {
    HeaderValue::from_static(
        "__Host-asterion_session=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict",
    )
}

fn random_token() -> Result<String, AuthError> {
    const HEX: &[u8] = b"0123456789abcdef";

    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(AuthError::internal)?;
    let mut token = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}

fn sha256(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recheck_requires_the_original_active_panel_session() {
        let pool = crate::database::connect("sqlite::memory:").await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let session = AdminSession {
            username: "fixture".into(),
            binding: [5; 32],
        };
        sqlx::query("INSERT INTO panel_admin (id, username, password_hash) VALUES (1, 'fixture', 'test-only')")
            .execute(&mut *conn).await.unwrap();
        sqlx::query("INSERT INTO panel_sessions (token_hash, admin_id, expires_at) VALUES (?, 1, unixepoch() + 30)")
            .bind(session.binding().as_slice()).execute(&mut *conn).await.unwrap();

        session.ensure_active(&mut conn).await.unwrap();
        sqlx::query("DELETE FROM panel_sessions WHERE token_hash = ?")
            .bind(session.binding().as_slice())
            .execute(&mut *conn)
            .await
            .unwrap();
        assert!(matches!(
            session.ensure_active(&mut conn).await,
            Err(AuthError::Unauthenticated)
        ));
    }
}
