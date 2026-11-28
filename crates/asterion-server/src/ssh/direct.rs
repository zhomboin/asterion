use tokio::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Direct {
    pub(super) auth: super::auth::Auth,
    host: String,
    user: Option<String>,
    port: Option<u16>,
}

impl Direct {
    pub(super) fn parse(value: &str) -> Option<Self> {
        let (user, host, port) = crate::connection_name::direct(value)?;
        Some(Self {
            auth: super::auth::Auth::Default,
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
