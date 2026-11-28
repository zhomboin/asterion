use crate::{model::Node, ssh_config};
use anyhow::{Context, Result, ensure};
use std::{
    pin::Pin,
    process::Stdio,
    task::{Context as PollContext, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    process::{Child, ChildStdin, ChildStdout},
};
pub struct Stream {
    _child: Child,
    read: ChildStdout,
    write: ChildStdin,
}
pub fn connect(chain: &str, target: &Node) -> Result<Stream> {
    let parts: Vec<_> = chain.split(',').collect();
    ensure!(
        parts.len() <= 16
            && parts.iter().all(|v| !v.is_empty()
                && !v.starts_with('-')
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"@.:-_[]".contains(&c))),
        "SSH 跳板配置无效"
    );
    let (last, prior) = parts.split_last().context("缺少跳板")?;
    let mut cmd = ssh_config::ssh();
    cmd.args([
        "-T",
        "-oBatchMode=yes",
        "-oStrictHostKeyChecking=yes",
        "-oUpdateHostKeys=no",
        "-oForwardAgent=no",
        "-oPermitLocalCommand=no",
        "-oControlMaster=no",
        "-oControlPath=none",
    ]);
    if !prior.is_empty() {
        cmd.arg("-J").arg(prior.join(","));
    }
    let (user, hostport) = last
        .split_once('@')
        .map_or((None, *last), |(u, h)| (Some(u), h));
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (h, p) = rest.split_once(']').context("跳板 IPv6 格式无效")?;
        (h, p.strip_prefix(':'))
    } else {
        hostport
            .split_once(':')
            .map_or((hostport, None), |(h, p)| (h, Some(p)))
    };
    ensure!(!host.is_empty() && !host.starts_with('-'), "跳板地址无效");
    if let Some(p) = port {
        let port = p.parse::<u16>()?;
        ensure!(port > 0, "跳板端口无效");
        cmd.arg("-p").arg(port.to_string());
    }
    if let Some(u) = user {
        cmd.arg("-l").arg(u);
    }
    let dest = if target.host.contains(':') {
        format!("[{}]:{}", target.host, target.port)
    } else {
        format!("{}:{}", target.host, target.port)
    };
    cmd.arg("-W").arg(dest).arg("--").arg(host);
    start(cmd)
}
pub fn start(mut cmd: tokio::process::Command) -> Result<Stream> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Stream {
        read: child.stdout.take().context("跳板输出不可用")?,
        write: child.stdin.take().context("跳板输入不可用")?,
        _child: child,
    })
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut PollContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut PollContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}
