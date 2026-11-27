use crate::ssh::{self, Session};
use iced::{
    Task,
    futures::{SinkExt, channel::mpsc::Sender},
};
use russh::ChannelMsg;
use std::time::Duration;
use tokio::{
    sync::mpsc,
    time::{Instant, sleep_until, timeout},
};

#[derive(Clone, Debug)]
pub enum Event {
    Open,
    Bytes(Vec<u8>),
    End(String),
}
pub enum Input {
    Bytes(Vec<u8>),
    Size(u16, u16),
}

pub fn run(session: Session, cols: u16, rows: u16, input: mpsc::Receiver<Input>) -> Task<Event> {
    let events = iced::stream::channel(16, async move |mut output| {
        let result = relay(session, cols, rows, input, &mut output).await;
        let reason = result
            .err()
            .map(|e| format!("终端已关闭 · {e}"))
            .unwrap_or_else(|| "终端已关闭".into());
        let _ = output.send(Event::End(reason)).await;
    });
    Task::run(events, std::convert::identity)
}

async fn relay(
    session: Session,
    cols: u16,
    rows: u16,
    mut input: mpsc::Receiver<Input>,
    output: &mut Sender<Event>,
) -> anyhow::Result<()> {
    let mut channel = tokio::select! {
        result = timeout(Duration::from_secs(15),ssh::terminal(&session,cols,rows)) => result??,
        _ = input.recv() => return Ok(()),
    };
    output.send(Event::Open).await?;
    let mut idle = Instant::now() + Duration::from_secs(120);
    let expires = Instant::now() + Duration::from_secs(900);
    loop {
        tokio::select! {
            _ = sleep_until(idle.min(expires)) => break,
            input = input.recv() => match input {
                Some(Input::Bytes(bytes)) => {
                    timeout(Duration::from_secs(5),channel.data_bytes(bytes)).await??;
                    idle=Instant::now()+Duration::from_secs(120);
                }
                Some(Input::Size(c,r)) => channel.window_change(c.into(),r.into(),0,0).await?,
                None => break,
            },
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data {data} | ChannelMsg::ExtendedData {data,..}) => {
                    if output.send(Event::Bytes(data.to_vec())).await.is_err() { break; }
                }
                Some(ChannelMsg::Failure) => anyhow::bail!("服务器拒绝终端请求"),
                Some(ChannelMsg::Close | ChannelMsg::Eof) | None => break,
                _ => {}
            }
        }
    }
    let _ = channel.close().await;
    Ok(())
}

pub fn paste_bytes(raw: &str, bracketed: bool) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        raw.len() <= 4096
            && !raw
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')),
        "粘贴内容过长或含控制字符"
    );
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    Ok(if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace('\n', "\r").into_bytes()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn paste_normalizes_crlf_and_rejects_escape_sequences() {
        assert_eq!(
            super::paste_bytes("一\r\n二", false).unwrap(),
            "一\r二".as_bytes()
        );
        assert!(super::paste_bytes("\x1b[201~", true).is_err());
    }
}
