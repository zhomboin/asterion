use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const LIMIT: usize = 16 * 1024;

pub async fn read<T: DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let length = stream.read_u32().await? as usize;
    if !(1..=LIMIT).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid terminal frame length",
        ));
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload).map_err(io::Error::other)
}

pub async fn write<T: Serialize>(stream: &mut UnixStream, frame: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(frame).map_err(io::Error::other)?;
    if payload.len() > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal frame exceeds limit",
        ));
    }
    stream.write_u32(payload.len() as u32).await?;
    stream.write_all(&payload).await
}
