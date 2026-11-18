use sqlx::{Sqlite, Transaction};

use super::{credentials::random_hex, error::ApiError};

pub(super) async fn resolve_id(
    tx: &mut Transaction<'_, Sqlite>,
    node_kind: &str,
    name: &str,
) -> Result<String, ApiError> {
    let kind = if node_kind == "wsl" {
        "windows"
    } else {
        "linux"
    };
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM hosts WHERE kind = ? AND name = ? ORDER BY created_at, id LIMIT 1",
    )
    .bind(kind)
    .bind(name)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing {
        return Ok(existing);
    }

    let id = random_hex::<16>()?;
    sqlx::query("INSERT INTO hosts (id, name, kind) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(kind)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}
