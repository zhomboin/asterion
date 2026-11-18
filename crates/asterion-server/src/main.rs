use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use asterion_server::{
    agent_bus::AgentBus, alerts, app, auth, config::Config, database, monitor, operations, trends,
};
use axum_server::Handle;
use axum_server::tls_rustls::{RustlsConfig, from_tcp_rustls};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .json()
        .init();

    let config = Config::from_env()?;
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("无法初始化 TLS 加密后端"))?;
    let tls = RustlsConfig::from_pem_file(&config.tls_cert_file, &config.tls_key_file)
        .await
        .context("无法加载中心服务的 TLS 证书或私钥")?;
    let listener = TcpListener::bind(config.listen_addr).context("无法绑定中心服务监听地址")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;

    let (pool, database_guard) = database::connect_center(&config.database_url).await?;
    auth::bootstrap_admin(
        &pool,
        config.admin_username.as_deref(),
        config.admin_password_file.as_deref(),
    )
    .await?;

    monitor::purge_metrics(&pool, config.retention_seconds)
        .await
        .context("无法清理过期采样")?;
    operations::recover(&pool)
        .await
        .context("无法恢复中断的运维任务")?;
    trends::purge(&pool, config.trend_retention_seconds)
        .await
        .context("无法清理过期趋势")?;
    let maintenance = maintain(pool.clone(), &config);
    let (stopping, shutdown) = watch::channel(false);
    let app = app::router(
        pool.clone(),
        config.web_dir.as_deref(),
        shutdown,
        Arc::new(
            AgentBus::new(config.sample_interval_secs)
                .with_terminal_enabled(config.terminal_enabled)
                .with_retention_days(
                    (config.retention_seconds / 86_400) as u64,
                    (config.trend_retention_seconds / 86_400) as u64,
                ),
        ),
    );
    let handle = Handle::new();
    let shutdown_handle = handle.clone();
    let signal = tokio::spawn(async move {
        shutdown_signal().await;
        stopping.send_replace(true);
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(5)));
    });

    tracing::info!(%address, "中心 HTTPS 服务已启动");
    let result = from_tcp_rustls(listener, tls)?
        .handle(handle)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("中心服务运行失败");
    signal.abort();
    maintenance.abort();
    pool.close().await;
    drop(database_guard);
    result
}

async fn shutdown_signal() {
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "无法监听 SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::pin!(terminate);
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::warn!(%error, "无法监听 Ctrl+C");
                terminate.await;
            }
        }
        () = &mut terminate => {}
    }
}

fn maintain(pool: sqlx::SqlitePool, config: &Config) -> tokio::task::JoinHandle<()> {
    let retention = config.retention_seconds;
    let trend_retention = config.trend_retention_seconds;
    let sample_interval = config.sample_interval_secs;
    tokio::spawn(async move {
        let hour = Duration::from_secs(3600);
        let mut retention_tick = tokio::time::interval_at(tokio::time::Instant::now() + hour, hour);
        retention_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut alert_tick = tokio::time::interval(Duration::from_secs(15));
        alert_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = retention_tick.tick() => {
                    if let Err(error) = monitor::purge_metrics(&pool, retention).await {
                        tracing::warn!(%error, "清理过期采样失败");
                    }
                    if let Err(error) = trends::purge(&pool, trend_retention).await {
                        tracing::warn!(%error, "清理过期趋势失败");
                    }
                }
                _ = alert_tick.tick() => {
                    if let Err(error) = alerts::evaluate(&pool, sample_interval).await {
                        tracing::warn!(%error, "评估告警失败");
                    }
                }
            }
        }
    })
}
