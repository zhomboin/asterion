mod bindings;
mod policy;
pub mod remote;
pub mod resume;
mod worker;

use crate::{model::Node, ssh::Session, store::Store};
use anyhow::{Context, Result, ensure};
use asterion_server::{
    alerts, app, auth, database, local, monitor, operations, service_monitors, ssh_bus::SshBus,
    trends,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;
use tower::ServiceExt;
use zeroize::Zeroizing;

pub use worker::Running;

pub struct Workspace {
    router: Router,
    token: Zeroizing<String>,
    pub bus: Arc<SshBus>,
    pool: SqlitePool,
    directory: PathBuf,
    _maintenance: Running,
    config: asterion_server::config::MonitoringConfig,
    _lock: File,
    pub expires_at: i64,
}

impl Workspace {
    pub async fn unlock(password: Zeroizing<String>) -> Result<Arc<Self>> {
        let directory = Store::directory()?;
        let nodes = Store::open()?.nodes()?;
        Self::open(directory, password, nodes).await
    }
    async fn open(
        directory: PathBuf,
        password: Zeroizing<String>,
        nodes: Vec<Node>,
    ) -> Result<Arc<Self>> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("workspace.lock"))?;
        lock.try_lock().context("工作区已在其他窗口打开")?;
        let pool = database::connect_path(&directory.join("workspace.sqlite3")).await?;
        auth::bootstrap_desktop_admin(&pool, password.to_string()).await?;
        let config = asterion_server::config::MonitoringConfig::from_env()?;
        let bus = Arc::new(
            SshBus::new(config.interval)
                .with_terminal_enabled(config.terminal_enabled)
                .with_retention_days(config.raw_days, config.trend_days),
        );
        let (_, stop) = watch::channel(false);
        let router = local::in_process(app::router(pool.clone(), None, stop, bus.clone()));
        let response = request(
            &router,
            "",
            Method::POST,
            "/auth/login",
            json!({"username":"local","password":password.as_str()}),
        )
        .await?;
        let token = response
            .headers()
            .get("x-asterion-local-session")
            .context("工作区口令错误")?
            .to_str()?
            .to_owned();
        let expires_at = sqlx::query_scalar::<_, i64>(
            "SELECT expires_at FROM panel_sessions WHERE token_hash=?",
        )
        .bind(Sha256::digest(token.as_bytes()).to_vec())
        .fetch_one(&pool)
        .await?;
        operations::recover(&pool).await?;
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS desktop_removed(profile TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS desktop_rebindings(old TEXT PRIMARY KEY,new TEXT NOT NULL);
            CREATE TRIGGER IF NOT EXISTS desktop_remove AFTER DELETE ON nodes WHEN OLD.ssh_profile IS NOT NULL
            BEGIN INSERT OR IGNORE INTO desktop_removed VALUES(OLD.ssh_profile); END;").execute(&pool).await?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        for n in nodes {
            let address = format!("ssh://{n}");
            sqlx::query("INSERT INTO nodes(id,name,kind,ssh_profile)
                SELECT CASE WHEN EXISTS(SELECT 1 FROM nodes WHERE id=?1) THEN lower(hex(randomblob(16))) ELSE ?1 END,?2,'linux',?3
                WHERE NOT EXISTS(SELECT 1 FROM nodes WHERE ssh_profile=?3)
                AND NOT EXISTS(SELECT 1 FROM desktop_removed WHERE profile=?3)
                AND NOT EXISTS(SELECT 1 FROM desktop_rebindings WHERE old=?3)")
                .bind(Self::id(&n)).bind(n.to_string()).bind(address).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        bindings::reconcile(&pool, &directory).await?;
        reconcile_removed(&pool, &directory).await?;
        let maintenance = maintain(pool.clone(), bus.clone(), config);
        Ok(Arc::new(Self {
            router,
            token: Zeroizing::new(token),
            pool,
            bus,
            directory,
            _maintenance: maintenance,
            config,
            _lock: lock,
            expires_at,
        }))
    }

    pub async fn api(&self, method: Method, path: &str, body: Value) -> Result<Value> {
        let remove = method == Method::DELETE
            && path
                .strip_prefix("/nodes/")
                .is_some_and(asterion_protocol::is_node_id);
        let response = request(&self.router, &self.token, method, path, body).await?;
        if remove && response.status().is_success() {
            reconcile_removed(&self.pool, &self.directory).await?;
        }
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024).await?;
        if !status.is_success() {
            let problem: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            anyhow::bail!(
                "{}（{}）",
                problem["message"].as_str().unwrap_or("请求失败"),
                status.as_u16()
            );
        }
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_slice(&bytes)?)
        }
    }

    pub fn id(node: &Node) -> String {
        format!("{:x}", Sha256::digest(format!("ssh://{node}").as_bytes()))[..32].into()
    }

    pub async fn attach(
        &self,
        node: Node,
        session: Session,
    ) -> Result<(String, Arc<Running>, remote::Remote)> {
        self.attach_as(node, session, None).await
    }
    pub async fn attach_as(
        &self,
        node: Node,
        session: Session,
        rebind: Option<(String, String)>,
    ) -> Result<(String, Arc<Running>, remote::Remote)> {
        self.api(Method::GET, "/me", Value::Null).await?;
        let address = format!("ssh://{node}");
        let remote = remote::Remote::new(session, &self.directory, &address).await?;
        let id = self.bind(&node, rebind).await?;
        let running = worker::start(
            self.pool.clone(),
            &self.bus,
            id.clone(),
            remote.clone(),
            self.config,
        );
        Ok((id, Arc::new(running), remote))
    }
}

async fn reconcile_removed(pool: &SqlitePool, dir: &std::path::Path) -> Result<()> {
    let profiles = sqlx::query_scalar::<_, String>("SELECT profile FROM desktop_removed")
        .fetch_all(pool)
        .await?;
    if !profiles.is_empty() {
        let mut store = Store::open_in(dir)?;
        for profile in profiles {
            store.forget(&profile)?;
            sqlx::query("DELETE FROM desktop_removed WHERE profile=?")
                .bind(profile)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

async fn request(
    router: &Router,
    token: &str,
    method: Method,
    path: &str,
    body: Value,
) -> Result<axum::response::Response> {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/api/v1{path}"))
        .header("x-asterion-csrf", "1");
    if !token.is_empty() {
        builder = builder.header("x-asterion-local-session", token);
    }
    let body = if body.is_null() {
        Body::empty()
    } else {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&body)?)
    };
    Ok(router.clone().oneshot(builder.body(body)?).await?)
}

fn maintain(
    pool: SqlitePool,
    bus: Arc<SshBus>,
    config: asterion_server::config::MonitoringConfig,
) -> Running {
    let task = tokio::spawn(async move {
        let mut iteration = 0u32;
        loop {
            let _ = alerts::evaluate(&pool, config.interval).await;
            let _ = service_monitors::poll_due(&pool, &bus).await;
            if iteration.is_multiple_of(240) {
                let _ = monitor::purge_metrics(&pool, (config.raw_days * 86400) as i64).await;
                let _ = trends::purge(&pool, (config.trend_days * 86400) as i64).await;
                let _ = service_monitors::purge(&pool, (config.trend_days * 86400) as i64).await;
            }
            iteration = iteration.wrapping_add(1);
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
    Running(task.abort_handle())
}

#[cfg(test)]
mod tests;
