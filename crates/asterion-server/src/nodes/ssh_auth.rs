use super::{credentials::random_hex, error::ApiError};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Sqlite, Transaction};

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Input {
    Default {},
    Key { identity_file: String },
    Password { password: Option<String> },
}

#[derive(FromRow, Serialize)]
pub(super) struct View {
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity_file: Option<String>,
}

pub(super) async fn view(pool: &sqlx::SqlitePool, id: &str) -> Result<Option<View>, ApiError> {
    Ok(
        sqlx::query_as("SELECT method, identity_file FROM node_ssh_auth WHERE node_id=?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

impl Input {
    pub(super) async fn save(
        self,
        tx: &mut Transaction<'_, Sqlite>,
        id: &str,
        same_destination: bool,
    ) -> Result<(), ApiError> {
        let (method, identity_file, password) = match self {
            Self::Default {} => {
                sqlx::query("DELETE FROM node_ssh_auth WHERE node_id=?")
                    .bind(id)
                    .execute(&mut **tx)
                    .await?;
                return Ok(());
            }
            Self::Key { identity_file } => {
                let file = identity_file.trim();
                if !(file.starts_with('/') || file.starts_with("~/"))
                    || file.len() > 4096
                    || file.chars().any(char::is_control)
                {
                    return Err(ApiError::INVALID_KEY_PATH);
                }
                let path = if let Some(relative) = file.strip_prefix("~/") {
                    std::env::home_dir()
                        .ok_or(ApiError::INVALID_KEY_PATH)?
                        .join(relative)
                } else {
                    std::path::PathBuf::from(file)
                };
                if !tokio::fs::metadata(path)
                    .await
                    .is_ok_and(|entry| entry.is_file())
                {
                    return Err(ApiError::INVALID_KEY_PATH);
                }
                ("key", Some(file.to_owned()), None)
            }
            Self::Password { password: None } if same_destination => {
                let saved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM node_ssh_auth WHERE node_id=? AND method='password')")
                    .bind(id).fetch_one(&mut **tx).await?;
                return if saved {
                    Ok(())
                } else {
                    Err(ApiError::INVALID_PASSWORD)
                };
            }
            Self::Password { password } => {
                let password = password
                    .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains(['\n', '\r', '\0']))
                    .ok_or(ApiError::INVALID_PASSWORD)?;
                ("password", None, Some(password))
            }
        };
        sqlx::query("INSERT INTO node_ssh_auth (node_id, method, identity_file, password, revision)
            VALUES (?, ?, ?, ?, ?) ON CONFLICT(node_id) DO UPDATE SET method=excluded.method,
            identity_file=excluded.identity_file, password=excluded.password, revision=excluded.revision, paused=0")
            .bind(id).bind(method).bind(identity_file).bind(password).bind(random_hex::<16>()?)
            .execute(&mut **tx).await?;
        Ok(())
    }
}
