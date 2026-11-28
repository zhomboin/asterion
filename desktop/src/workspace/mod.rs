pub mod remote;
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
    _lock: File,
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
        let bus = Arc::new(SshBus::new(15).with_terminal_enabled(true));
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
        operations::recover(&pool).await?;
        // 仅在首次打开管理库时导入快捷连接，后续移除节点不会被自动恢复。
        sqlx::query("CREATE TABLE IF NOT EXISTS desktop_import (version INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await?;
        let mut tx = pool.begin().await?;
        if sqlx::query("INSERT OR IGNORE INTO desktop_import VALUES(1)")
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1
        {
            for node in nodes {
                sqlx::query(
                    "INSERT OR IGNORE INTO nodes(id,name,kind,ssh_profile) VALUES(?,?,'linux',?)",
                )
                .bind(Self::id(&node))
                .bind(node.to_string())
                .bind(format!("ssh://{node}"))
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS desktop_removed(profile TEXT PRIMARY KEY);
            CREATE TRIGGER IF NOT EXISTS desktop_remove AFTER DELETE ON nodes WHEN OLD.ssh_profile IS NOT NULL
            BEGIN INSERT OR IGNORE INTO desktop_removed VALUES(OLD.ssh_profile); END;").execute(&pool).await?;
        reconcile_removed(&pool, &directory).await?;
        let maintenance = maintain(pool.clone(), bus.clone());
        Ok(Arc::new(Self {
            router,
            token: Zeroizing::new(token),
            pool,
            bus,
            directory,
            _maintenance: maintenance,
            _lock: lock,
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
        let id = Self::id(&node);
        let address = format!("ssh://{node}");
        let remote = remote::Remote::new(session, &self.directory, &address).await?;
        sqlx::query("INSERT INTO nodes(id,name,kind,ssh_profile) VALUES(?,?,'linux',?) ON CONFLICT(id) DO NOTHING")
            .bind(&id).bind(node.to_string()).bind(&address).execute(&self.pool).await?;
        let matching = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=? AND ssh_profile=?)",
        )
        .bind(&id)
        .bind(&address)
        .fetch_one(&self.pool)
        .await?;
        ensure!(matching, "已保存节点的 SSH 身份发生变化");
        let running = worker::start(self.pool.clone(), &self.bus, id.clone(), remote.clone());
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

fn maintain(pool: SqlitePool, bus: Arc<SshBus>) -> Running {
    let task = tokio::spawn(async move {
        let mut iteration = 0u32;
        loop {
            let _ = alerts::evaluate(&pool, 15).await;
            let _ = service_monitors::poll_due(&pool, &bus).await;
            if iteration.is_multiple_of(240) {
                let _ = monitor::purge_metrics(&pool, 7 * 86400).await;
                let _ = trends::purge(&pool, 90 * 86400).await;
                let _ = service_monitors::purge(&pool, 90 * 86400).await;
            }
            iteration = iteration.wrapping_add(1);
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
    Running(task.abort_handle())
}

#[cfg(test)]
mod tests;
