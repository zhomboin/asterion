use std::net::Ipv6Addr;

pub fn valid_profile(name: &str) -> bool {
    name.len() <= 64
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub(crate) fn direct(value: &str) -> Option<(Option<String>, &str, Option<u16>)> {
    let destination = value.strip_prefix("ssh://")?;
    let (user, address) = match destination.split_once('@') {
        Some((user, address)) if valid_profile(user) => (Some(user.to_owned()), address),
        Some(_) => return None,
        None => (None, destination),
    };
    let (host, port) = if let Some(ip) = address.strip_prefix('[') {
        let (ip, rest) = ip.split_once(']')?;
        ip.parse::<Ipv6Addr>().ok()?;
        (ip, rest)
    } else {
        let end = address.find(':').unwrap_or(address.len());
        let (host, rest) = address.split_at(end);
        if host.len() > 253
            || !host.as_bytes().first()?.is_ascii_alphanumeric()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        {
            return None;
        }
        (host, rest)
    };
    let port = if port.is_empty() {
        None
    } else {
        let digits = port.strip_prefix(':')?;
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(digits.parse::<u16>().ok().filter(|p| *p > 0)?)
    };
    Some((user, host, port))
}

pub fn valid_connection(value: &str, local: bool) -> bool {
    valid_profile(value) || local && direct(value).is_some()
}
