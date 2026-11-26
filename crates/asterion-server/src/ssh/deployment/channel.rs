use super::super::config::Target;
use anyhow::{Context, Result, ensure};
use std::process::Stdio;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
};

pub(super) async fn call(
    target: &Target,
    command: &str,
    upload: Option<(File, &str, &str)>,
) -> Result<(i32, String)> {
    let mut child = target
        .command(false)
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动 SSH 客户端")?;
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    if let Some((_, _, revision)) = &upload {
        let mut ready = String::new();
        let received = (&mut output)
            .take(80)
            .read_line(&mut ready)
            .await
            .context("SSH 部署握手中断")?;
        if received == 0 {
            drop(input);
            let status = child.wait().await.context("无法读取 SSH 状态")?;
            return Ok((status.code().unwrap_or(-1), String::new()));
        }
        ensure!(
            ready.trim_end() == format!("READY {revision}"),
            "SSH 目标未确认，部署已停止"
        );
    }
    let send = async move {
        if let Some((mut file, secret, _)) = upload {
            input.write_all(secret.as_bytes()).await?;
            input.write_all(b"\n").await?;
            tokio::io::copy(&mut file, &mut input).await?;
        }
        input.shutdown().await
    };
    let receive = async move {
        let mut bytes = Vec::new();
        output.take(4097).read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 4096, "SSH 部署响应过大");
        Ok::<_, anyhow::Error>(String::from_utf8(bytes)?)
    };
    let (sent, output) = tokio::join!(send, receive);
    let output = output.context("无法读取 SSH 部署结果")?;
    let status = child.wait().await.context("无法读取 SSH 部署状态")?;
    if status.success() {
        sent.context("SSH 传输中断")?;
    }
    Ok((status.code().unwrap_or(-1), output))
}
