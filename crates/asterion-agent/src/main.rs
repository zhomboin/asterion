mod client;
mod config;
mod identity;
mod metadata;
mod terminal_proxy;
mod terminal_wire;
mod tls;
mod transport;
mod worker;

use std::env;
use std::process::ExitCode;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "用法\n  asterion-agent\n  asterion-agent --version\n  asterion-agent --help";

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).take(2).collect();
    match arguments.as_slice() {
        [] => match start() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Agent 启动失败：{error:#}");
                ExitCode::FAILURE
            }
        },
        [argument] if argument == "--version" || argument == "-V" => {
            println!("asterion-agent {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [argument] if argument == "--help" || argument == "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn start() -> Result<()> {
    let configuration = config::Config::from_env()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .json()
        .init();

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("无法监听 SIGTERM")?;
    tokio::select! {
        result = client::run(configuration) => result,
        result = tokio::signal::ctrl_c() => result.context("无法监听 Ctrl+C"),
        _ = terminate.recv() => Ok(()),
    }
}
