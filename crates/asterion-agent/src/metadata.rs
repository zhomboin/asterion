use std::env;
use std::fs::File;
use std::io::{self, Read};

use asterion_protocol::Hello;

pub fn collect() -> Hello {
    let os = read_text("/etc/os-release")
        .or_else(|_| read_text("/usr/lib/os-release"))
        .ok()
        .and_then(|contents| pretty_name(&contents))
        .unwrap_or_else(|| "Linux".to_owned());
    let wsl_distro = env::var("WSL_DISTRO_NAME")
        .ok()
        .and_then(|name| bounded_text(&name, 128));

    Hello {
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        os,
        arch: env::consts::ARCH.to_owned(),
        wsl_distro,
    }
}

fn read_text(path: &str) -> io::Result<String> {
    let mut contents = String::new();
    File::open(path)?
        .take(16 * 1024)
        .read_to_string(&mut contents)?;
    Ok(contents)
}

fn pretty_name(contents: &str) -> Option<String> {
    let value = contents
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?
        .trim();
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value);
    bounded_text(value, 256)
}

fn bounded_text(value: &str, max: usize) -> Option<String> {
    let sanitized: String = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(max)
        .collect();
    (!sanitized.is_empty()).then_some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_os_metadata_without_interpreting_shell_syntax() {
        assert_eq!(
            pretty_name("NAME=Ubuntu\nPRETTY_NAME=\"Ubuntu 24.04 LTS\""),
            Some("Ubuntu 24.04 LTS".to_owned())
        );
        assert_eq!(
            pretty_name("PRETTY_NAME='Linux $(ignored)'"),
            Some("Linux $(ignored)".to_owned())
        );
        assert_eq!(pretty_name("NAME=Ubuntu"), None);
        assert_eq!(bounded_text("\0\t宇宙", 1), Some("宇".to_owned()));
    }
}
