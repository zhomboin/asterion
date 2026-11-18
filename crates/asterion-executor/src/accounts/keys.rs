use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ExecError;

const MAX_KEYS: usize = 128;
const MAX_LINE_BYTES: usize = 8192;

#[derive(Serialize)]
pub(crate) struct PublicKey {
    #[serde(rename = "type")]
    key_type: String,
    pub(crate) fingerprint: String,
    comment: String,
}

pub(crate) fn inventory(contents: &[u8]) -> Result<Value, ExecError> {
    let mut parsed = contents
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| parse(line, true));
    let keys: Vec<_> = parsed.by_ref().take(MAX_KEYS).collect();
    Ok(json!({"keys":keys, "truncated":parsed.next().is_some()}))
}

pub(crate) fn parse(line: &str, permit_options: bool) -> Option<PublicKey> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.len() > MAX_LINE_BYTES {
        return None;
    }
    let (first, remainder) = field(line)?;
    let (key_type, remainder) = if supported(first) {
        (first, remainder)
    } else if permit_options {
        field(remainder)?
    } else {
        return None;
    };
    if !supported(key_type) {
        return None;
    }
    let (encoded, raw_comment) = field(remainder)?;
    let blob = STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .ok()?;
    validate_blob(&blob, key_type)?;
    let comment = raw_comment
        .chars()
        .filter(|ch| !ch.is_control())
        .take(256)
        .collect::<String>();
    Some(PublicKey {
        key_type: key_type.to_owned(),
        fingerprint: format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob))),
        comment: asterion_collector::redact_output_line(&comment).to_owned(),
    })
}

fn field(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_ascii_whitespace() && !quoted {
            return Some((&input[..index], input[index..].trim_start()));
        }
    }
    (!quoted).then_some((input, ""))
}

fn supported(value: &str) -> bool {
    matches!(
        value,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "sk-ssh-ed25519@openssh.com"
            | "sk-ecdsa-sha2-nistp256@openssh.com"
    )
}

fn next_part<'a>(value: &mut &'a [u8]) -> Option<&'a [u8]> {
    let count = u32::from_be_bytes(value.get(..4)?.try_into().ok()?) as usize;
    let end = count.checked_add(4)?;
    let part = value.get(4..end)?;
    *value = &value[end..];
    Some(part)
}

fn validate_blob(mut body: &[u8], key_type: &str) -> Option<()> {
    if next_part(&mut body)? != key_type.as_bytes() {
        return None;
    }
    match key_type {
        "ssh-rsa" => {
            let exponent = next_part(&mut body)?;
            let modulus = next_part(&mut body)?;
            if exponent.is_empty()
                || exponent.len() > 8
                || modulus.len() < 256
                || modulus.len() > 1025
            {
                return None;
            }
        }
        "ssh-ed25519" | "sk-ssh-ed25519@openssh.com" => {
            if next_part(&mut body)?.len() != 32 {
                return None;
            }
        }
        _ => {
            let curve = std::str::from_utf8(next_part(&mut body)?).ok()?;
            let expected = match curve {
                "nistp256" => 65,
                "nistp384" => 97,
                "nistp521" => 133,
                _ => return None,
            };
            if !key_type.contains(curve) || next_part(&mut body)?.len() != expected {
                return None;
            }
        }
    }
    if key_type.starts_with("sk-") && next_part(&mut body)?.is_empty() {
        return None;
    }
    body.is_empty().then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed25519() -> String {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11_u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32_u32.to_be_bytes());
        blob.extend_from_slice(&[1; 32]);
        format!("ssh-ed25519 {} fixture", STANDARD.encode(blob))
    }

    #[test]
    fn fingerprints_independent_of_comments_and_rejects_options_on_add() {
        let key = ed25519();
        let existing = format!("command=\"printf hello\",restrict {key}");
        let first = parse(&key, false).unwrap();
        let second = parse(&existing, true).unwrap();
        assert_eq!(first.fingerprint.len(), 50);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(second.comment, "fixture");
        assert!(parse(&existing, false).is_none());
        assert!(parse("ssh-ed25519 YWJj", false).is_none());
        assert!(parse("-----BEGIN OPENSSH PRIVATE KEY-----", true).is_none());
    }
}
