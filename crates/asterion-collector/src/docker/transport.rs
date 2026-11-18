use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use socket2::{Domain, SockAddr, Socket, Type};

use crate::CollectorError;

const HEADERS_LIMIT: usize = 16 * 1024;
const MAX_CHUNK_LINE: usize = 128;

pub(super) fn connect(path: &Path) -> Result<UnixStream, CollectorError> {
    if !path.is_absolute() {
        return Err(CollectorError::INVALID);
    }
    let address = SockAddr::unix(path).map_err(io_error)?;
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None).map_err(io_error)?;
    socket
        .connect_timeout(&address, Duration::from_millis(450))
        .map_err(io_error)?;
    let fd = socket.into_raw_fd();
    // The socket descriptor is moved here and is not owned by socket2 after into_raw_fd.
    Ok(unsafe { UnixStream::from_raw_fd(fd) })
}

pub(super) fn io_error(error: io::Error) -> CollectorError {
    use io::ErrorKind as Kind;
    match error.kind() {
        Kind::PermissionDenied => CollectorError::PERMISSION,
        Kind::TimedOut | Kind::WouldBlock => CollectorError::TIMEOUT,
        Kind::InvalidInput => CollectorError::INVALID,
        _ => CollectorError::UNAVAILABLE,
    }
}

pub(super) struct Output {
    pub status: u16,
    pub content: Vec<u8>,
    pub truncated: bool,
}

pub(super) fn get(
    socket: &Path,
    endpoint: &str,
    hard_limit: usize,
    partial: bool,
    deadline: Instant,
) -> Result<Output, CollectorError> {
    let mut stream = connect(socket)?;
    let request =
        format!("GET {endpoint} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let timeout = remaining(deadline)?;
    stream.set_write_timeout(Some(timeout)).map_err(io_error)?;
    stream.write_all(request.as_bytes()).map_err(io_error)?;

    let (data, oversized) = read(&mut stream, hard_limit + HEADERS_LIMIT, deadline)?;
    let mut headers = [httparse::EMPTY_HEADER; 40];
    let mut response = httparse::Response::new(&mut headers);
    let header_len = match response.parse(&data) {
        Ok(httparse::Status::Complete(length)) if length <= HEADERS_LIMIT => length,
        _ => return Err(CollectorError::FAILED),
    };
    let status = response.code.ok_or(CollectorError::FAILED)?;
    let chunked = response.headers.iter().any(|header| {
        header.name.eq_ignore_ascii_case("transfer-encoding")
            && header.value.eq_ignore_ascii_case(b"chunked")
    });
    let expected_length = response.headers.iter().find_map(|header| {
        if !header.name.eq_ignore_ascii_case("content-length") {
            return None;
        }
        std::str::from_utf8(header.value)
            .ok()?
            .parse::<usize>()
            .ok()
    });

    let (mut content, mut truncated) = if chunked {
        decode_chunks(&data[header_len..], hard_limit, oversized)?
    } else {
        let body = &data[header_len..];
        if !oversized && expected_length.is_some_and(|length| length != body.len()) {
            return Err(CollectorError::FAILED);
        }
        (
            body[..body.len().min(hard_limit)].to_vec(),
            body.len() > hard_limit || oversized,
        )
    };
    if content.len() > hard_limit {
        content.truncate(hard_limit);
        truncated = true;
    }
    if truncated && !partial {
        return Err(CollectorError::LARGE);
    }
    Ok(Output {
        status,
        content,
        truncated,
    })
}

fn read(
    stream: &mut UnixStream,
    limit: usize,
    deadline: Instant,
) -> Result<(Vec<u8>, bool), CollectorError> {
    let mut data = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 8192];
    while data.len() <= limit {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(io_error)?;
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_error(error)),
        };
        if count == 0 {
            return Ok((data, false));
        }
        data.extend_from_slice(&buffer[..count]);
    }
    data.truncate(limit);
    Ok((data, true))
}

fn remaining(deadline: Instant) -> Result<Duration, CollectorError> {
    let duration = deadline.saturating_duration_since(Instant::now());
    if duration.is_zero() {
        return Err(CollectorError::TIMEOUT);
    }
    Ok(duration)
}

fn decode_chunks(
    mut source: &[u8],
    limit: usize,
    oversized: bool,
) -> Result<(Vec<u8>, bool), CollectorError> {
    let mut output = Vec::new();
    loop {
        let end = source
            .windows(2)
            .take(MAX_CHUNK_LINE)
            .position(|bytes| bytes == b"\r\n");
        let Some(end) = end else {
            if oversized {
                return Ok((output, true));
            }
            return Err(CollectorError::FAILED);
        };
        let header = std::str::from_utf8(&source[..end]).map_err(|_| CollectorError::FAILED)?;
        let size = usize::from_str_radix(header.split(';').next().unwrap_or_default().trim(), 16)
            .map_err(|_| CollectorError::FAILED)?;
        source = &source[end + 2..];
        if size == 0 {
            return Ok((output, false));
        }
        let count = size.min(source.len()).min(limit - output.len());
        output.extend_from_slice(&source[..count]);
        if count < size {
            if oversized || output.len() == limit {
                return Ok((output, true));
            }
            return Err(CollectorError::FAILED);
        }
        if source.get(count..count + 2) != Some(b"\r\n") {
            return Err(CollectorError::FAILED);
        }
        source = &source[count + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_response_is_bounded_and_rejects_missing_parts() {
        let raw = b"5\r\nfirst\r\n6\r\nsecond\r\n0\r\n\r\n";
        assert_eq!(
            decode_chunks(raw, 16, false).unwrap(),
            (b"firstsecond".to_vec(), false)
        );
        assert_eq!(
            decode_chunks(raw, 8, false).unwrap(),
            (b"firstsec".to_vec(), true)
        );
        assert!(decode_chunks(b"7\r\nshort", 20, false).is_err());
    }
}
