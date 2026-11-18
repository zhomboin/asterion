use asterion_protocol::Hello;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};

use super::error::ApiError;

const ENROLLMENT_TTL_SECS: i64 = 10 * 60;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

type Fingerprint = [u8; 32];

#[derive(Serialize)]
pub(super) struct IssuedEnrollment {
    pub node_id: String,
    pub token: String,
    pub expires_at: i64,
}

pub(super) struct Candidate {
    pub node_id: String,
    device: Fingerprint,
    enrollment: Option<Fingerprint>,
}

pub(super) struct Connection {
    pub node_id: String,
    device: Fingerprint,
    generation: i64,
}

pub(super) fn random_hex<const N: usize>() -> Result<String, ApiError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::internal("os_random"))?;
    let mut encoded = String::with_capacity(N * 2);
    for byte in bytes {
        encoded.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX_DIGITS[usize::from(byte & 0xf)]));
    }
    Ok(encoded)
}

fn fingerprint(token: &str) -> Fingerprint {
    Sha256::digest(token.as_bytes()).into()
}

pub(super) async fn revoke(
    tx: &mut Transaction<'_, Sqlite>,
    node_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agent_credentials SET revoked_at = unixepoch()
         WHERE node_id = ? AND revoked_at IS NULL",
    )
    .bind(node_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "UPDATE node_enrollment_tokens SET revoked_at = unixepoch()
         WHERE node_id = ? AND revoked_at IS NULL AND consumed_at IS NULL",
    )
    .bind(node_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn issue(
    tx: &mut Transaction<'_, Sqlite>,
    node_id: &str,
) -> Result<IssuedEnrollment, ApiError> {
    let token = random_hex::<32>()?;
    let fingerprint = fingerprint(&token);
    revoke(tx, node_id).await?;

    let expires_at = sqlx::query_scalar(
        "INSERT INTO node_enrollment_tokens (node_id, fingerprint, issued_at, expires_at)
         VALUES (?, ?, unixepoch(), unixepoch() + ?)
         ON CONFLICT(node_id) DO UPDATE SET
             fingerprint = excluded.fingerprint,
             issued_at = excluded.issued_at,
             expires_at = excluded.expires_at,
             consumed_at = NULL,
             revoked_at = NULL
         RETURNING expires_at",
    )
    .bind(node_id)
    .bind(fingerprint.as_slice())
    .bind(ENROLLMENT_TTL_SECS)
    .fetch_one(&mut **tx)
    .await?;

    Ok(IssuedEnrollment {
        node_id: node_id.to_owned(),
        token,
        expires_at,
    })
}

pub(super) async fn authenticate(
    pool: &SqlitePool,
    bearer: &str,
    enrollment: Option<&str>,
) -> Result<Candidate, ApiError> {
    let device = fingerprint(bearer);
    if let Some((node_id, revoked_at)) = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT node_id, revoked_at FROM agent_credentials WHERE fingerprint = ?",
    )
    .bind(device.as_slice())
    .fetch_optional(pool)
    .await?
    {
        if revoked_at.is_some() {
            return Err(ApiError::UNAUTHORIZED);
        }
        return Ok(Candidate {
            node_id,
            device,
            enrollment: None,
        });
    }

    let enrollment = fingerprint(enrollment.ok_or(ApiError::UNAUTHORIZED)?);
    let node_id = sqlx::query_scalar(
        "SELECT node_id FROM node_enrollment_tokens WHERE fingerprint = ?
         AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at > unixepoch()",
    )
    .bind(enrollment.as_slice())
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::UNAUTHORIZED)?;

    Ok(Candidate {
        node_id,
        device,
        enrollment: Some(enrollment),
    })
}

pub(super) async fn connect(
    pool: &SqlitePool,
    candidate: Candidate,
    hello: &Hello,
) -> Result<(Connection, i64), ApiError> {
    let mut tx = pool.begin().await?;
    let generation = match candidate.enrollment {
        Some(enrollment) => {
            enroll(&mut tx, &candidate.node_id, &candidate.device, &enrollment).await?
        }
        None => sqlx::query_scalar(
            "UPDATE agent_credentials SET connection_generation = connection_generation + 1
                 WHERE fingerprint = ? AND node_id = ? AND revoked_at IS NULL
                 RETURNING connection_generation",
        )
        .bind(candidate.device.as_slice())
        .bind(&candidate.node_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::UNAUTHORIZED)?,
    };

    let last_seen_at = sqlx::query_scalar(
        "UPDATE nodes SET agent_version = ?, os = ?, arch = ?,
             wsl_distro = CASE WHEN kind = 'wsl' THEN ? ELSE NULL END,
             last_seen_at = unixepoch(), updated_at = unixepoch()
         WHERE id = ? RETURNING last_seen_at",
    )
    .bind(&hello.agent_version)
    .bind(&hello.os)
    .bind(&hello.arch)
    .bind(&hello.wsl_distro)
    .bind(&candidate.node_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok((
        Connection {
            node_id: candidate.node_id,
            device: candidate.device,
            generation,
        },
        last_seen_at,
    ))
}

async fn enroll(
    tx: &mut Transaction<'_, Sqlite>,
    node_id: &str,
    device: &Fingerprint,
    enrollment: &Fingerprint,
) -> Result<i64, ApiError> {
    let consumed = sqlx::query(
        "UPDATE node_enrollment_tokens SET consumed_at = unixepoch()
         WHERE node_id = ? AND fingerprint = ?
         AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at > unixepoch()",
    )
    .bind(node_id)
    .bind(enrollment.as_slice())
    .execute(&mut **tx)
    .await?;
    if consumed.rows_affected() == 0 {
        return Err(ApiError::UNAUTHORIZED);
    }

    sqlx::query_scalar(
        "INSERT INTO agent_credentials (fingerprint, node_id, created_at, connection_generation)
         VALUES (?, ?, unixepoch(), 1)
         ON CONFLICT DO NOTHING
         RETURNING connection_generation",
    )
    .bind(device.as_slice())
    .bind(node_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApiError::UNAUTHORIZED)
}

pub(super) async fn heartbeat(pool: &SqlitePool, connection: &Connection) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "UPDATE nodes SET last_seen_at = unixepoch()
         WHERE id = ? AND EXISTS (
             SELECT 1 FROM agent_credentials
             WHERE fingerprint = ? AND node_id = nodes.id
               AND connection_generation = ? AND revoked_at IS NULL
         ) RETURNING last_seen_at",
    )
    .bind(&connection.node_id)
    .bind(connection.device.as_slice())
    .bind(connection.generation)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::UNAUTHORIZED)
}

pub(super) async fn ensure_current(
    conn: &mut sqlx::SqliteConnection,
    connection: &Connection,
) -> Result<(), ApiError> {
    let current: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM agent_credentials
         WHERE fingerprint = ? AND node_id = ?
           AND connection_generation = ? AND revoked_at IS NULL)",
    )
    .bind(connection.device.as_slice())
    .bind(&connection.node_id)
    .bind(connection.generation)
    .fetch_one(conn)
    .await?;
    if current {
        Ok(())
    } else {
        Err(ApiError::UNAUTHORIZED)
    }
}
