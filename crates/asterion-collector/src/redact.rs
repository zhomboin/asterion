const REDACTED: &str = "[已隐藏可能包含凭证的内容]";

pub(crate) fn arguments(arguments: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut hide_next = false;
    arguments
        .into_iter()
        .map(|value| {
            let secret_value = hide_next;
            hide_next = value.starts_with('-') && !value.contains('=') && sensitive(&value);
            if secret_value || sensitive(&value) || has_url_userinfo(&value) {
                REDACTED.to_owned()
            } else {
                value
            }
        })
        .collect()
}

pub fn line(value: &str) -> &str {
    if sensitive(value) || has_url_userinfo(value) {
        REDACTED
    } else {
        value
    }
}

fn sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "passphrase",
        "authorization:",
        "bearer ",
        "token",
        "secret",
        "api_key",
        "apikey",
        "api-key",
        "private_key",
        "client-key",
        "key=",
        "--key",
        "credential",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn has_url_userinfo(value: &str) -> bool {
    value.split("://").skip(1).any(|url| {
        url.split(['/', ' ', '?', '#'])
            .next()
            .is_some_and(|authority| authority.contains('@'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_option_values_and_url_credentials() {
        let sample = [
            "app",
            "--token",
            "sensitive-value",
            "unix://local",
            "https://user:secret@example.org/",
        ];
        let redacted = arguments(sample.into_iter().map(str::to_owned));
        assert_eq!(
            redacted,
            ["app", REDACTED, REDACTED, "unix://local", REDACTED]
        );
        assert_eq!(line("Authorization: Bearer 123"), REDACTED);
        assert_eq!(line("normal message"), "normal message");
    }
}
