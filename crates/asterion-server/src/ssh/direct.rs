use std::net::Ipv6Addr;
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Direct {
    host: String,
    user: Option<String>,
    port: Option<u16>,
}

impl Direct {
    pub(super) fn parse(value: &str) -> Option<Self> {
        let destination = value.strip_prefix("ssh://")?;
        let (user, address) = match destination.split_once('@') {
            Some((user, address)) if super::config::valid_profile(user) => {
                (Some(user.to_owned()), address)
            }
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
        Some(Self {
            host: host.to_owned(),
            user,
            port,
        })
    }

    pub(super) fn configure<'a>(&'a self, cmd: &mut Command) -> &'a str {
        if let Some(sock) = std::env::var_os("SSH_AUTH_SOCK") {
            cmd.env("SSH_AUTH_SOCK", sock);
        }
        if let Some(file) = std::env::var_os("ASTERION_LOCAL_SSH_CONFIG") {
            cmd.arg("-F").arg(file);
        }
        cmd.args([
            "-oPermitLocalCommand=no",
            "-oRemoteCommand=none",
            "-oControlPath=none",
            "-oControlMaster=no",
            "-oControlPersist=no",
            "-oForkAfterAuthentication=no",
            "-oForwardX11=no",
        ]);
        if let Some(port) = self.port {
            cmd.arg("-p").arg(port.to_string());
        }
        if let Some(user) = &self.user {
            cmd.arg("-l").arg(user);
        }
        &self.host
    }
}

#[cfg(test)]
mod tests {
    use super::Direct;
    #[test]
    fn validates_destinations_and_preserves_ssh_defaults() {
        let target = Direct::parse("ssh://my-host").unwrap();
        assert_eq!((target.user, target.port), (None, None));
        assert_eq!(
            Direct::parse("ssh://name@[::1]:2200").unwrap().port,
            Some(2200)
        );
        for value in [
            "ssh://-F",
            "ssh://x@-o",
            "ssh://x:0",
            "ssh://x:65536",
            "ssh://x:",
            "ssh://a@b@c",
            "ssh://a b",
            "ssh://a;b",
            "ssh://bad/name",
            "ssh://[x]",
            "ssh://",
        ] {
            assert!(Direct::parse(value).is_none(), "{value}");
        }
    }
}
