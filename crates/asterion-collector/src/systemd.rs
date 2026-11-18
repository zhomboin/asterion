use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CollectorError, observed_at, redact, truncate_utf8};

const MAX_UNITS: usize = 128;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_STDOUT: usize = 512 * 1024;
const MAX_STDERR: usize = 8 * 1024;
const PROPERTIES: &str = "--property=Id,LoadState,ActiveState,SubState,Description,ExecStart,WorkingDirectory,FragmentPath";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListArgs {
    q: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetailArgs {
    unit: String,
}

#[derive(Serialize)]
struct Unit {
    name: String,
    active: String,
    sub: String,
    description: String,
    exec_start: Option<String>,
    working_directory: Option<String>,
    fragment_path: Option<String>,
    observed_at: u64,
    source: &'static str,
    verified: bool,
}

pub(crate) fn list(args: ListArgs) -> Result<Value, CollectorError> {
    if args.q.as_ref().is_some_and(|value| value.len() > 256) {
        return Err(CollectorError::INVALID);
    }

    match collect(args.q.as_deref()) {
        Ok((units, truncated)) => {
            Ok(json!({"status":"available", "units":units,"truncated":truncated}))
        }
        Err(CollectorError {
            code: "unavailable",
            ..
        }) => Ok(outcome("unavailable")),
        Err(CollectorError {
            code: "permission_denied",
            ..
        }) => Ok(outcome("permission_denied")),
        Err(_) => Ok(outcome("error")),
    }
}

fn outcome(status: &str) -> Value {
    json!({"status":status,"units":[],"truncated":false})
}

fn collect(query: Option<&str>) -> Result<(Vec<Unit>, bool), CollectorError> {
    let filter = query.unwrap_or_default().trim().to_lowercase();
    let loaded = systemctl(&[
        "list-units",
        "--all",
        "--type=service",
        "--plain",
        "--no-legend",
    ])?;
    let mut units = BTreeMap::new();
    for line in loaded.lines() {
        if let Some(unit) = parse_listed_unit(line) {
            units.insert(unit.name.clone(), unit);
        }
    }
    let installed = systemctl(&["list-unit-files", "--type=service", "--no-legend"]);
    if let Ok(installed) = installed {
        for name in installed
            .lines()
            .filter_map(|line| line.split_whitespace().next())
        {
            if valid_name(name) && !name.ends_with("@.service") {
                units
                    .entry(name.to_owned())
                    .or_insert_with(|| base_unit(name));
            }
        }
    }

    units.retain(|name, _| filter.is_empty() || name.to_lowercase().contains(&filter));
    let truncated = units.len() > MAX_UNITS;
    let mut units: BTreeMap<_, _> = units.into_iter().take(MAX_UNITS).collect();
    if !units.is_empty() {
        let mut args = vec!["show", "--all", PROPERTIES, "--"];
        args.extend(units.keys().map(String::as_str));
        if let Ok(properties) = systemctl(&args) {
            for unit in parse_properties(&properties) {
                if let Some(previous) = units.get_mut(&unit.name) {
                    *previous = unit;
                }
            }
        }
    }

    // Other unit metadata remains accessible through the dedicated detail query.
    let mut units: Vec<_> = units.into_values().collect();
    let mut limited = truncated;
    while serde_json::to_vec(&units)
        .map_err(|_| CollectorError::FAILED)?
        .len()
        > crate::MAX_QUERY_BYTES - 512
    {
        units.pop();
        limited = true;
    }
    Ok((units, limited))
}

pub(crate) fn detail(args: DetailArgs) -> Result<Value, CollectorError> {
    if !valid_name(&args.unit) || args.unit.ends_with("@.service") {
        return Err(CollectorError::INVALID);
    }
    let properties = systemctl(&["show", "--all", PROPERTIES, "--", &args.unit])?;
    let unit = parse_properties(&properties)
        .into_iter()
        .next()
        .ok_or(CollectorError::NOT_FOUND)?;
    serde_json::to_value(unit).map_err(|_| CollectorError::FAILED)
}

fn base_unit(name: &str) -> Unit {
    Unit {
        name: name.to_owned(),
        active: "unknown".into(),
        sub: "unknown".into(),
        description: String::new(),
        exec_start: None,
        working_directory: None,
        fragment_path: None,
        observed_at: observed_at(),
        source: "systemd",
        verified: false,
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    name.len() <= 255
        && name.ends_with(".service")
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name.bytes().all(|value| {
            value.is_ascii_alphanumeric()
                || matches!(value, b':' | b'_' | b'\\' | b'-' | b'.' | b'@')
        })
}

fn parse_listed_unit(line: &str) -> Option<Unit> {
    let mut columns = line.split_whitespace();
    let name = columns.next()?;
    if !valid_name(name) {
        return None;
    }
    let mut unit = base_unit(name);
    unit.active = columns.nth(1)?.to_owned();
    unit.sub = columns.next()?.to_owned();
    unit.description = truncate_utf8(&columns.collect::<Vec<_>>().join(" "), 512).to_owned();
    Some(unit)
}

fn parse_properties(contents: &str) -> Vec<Unit> {
    contents
        .split("\n\n")
        .filter_map(|entry| {
            let fields: BTreeMap<_, _> = entry
                .lines()
                .filter_map(|line| line.split_once('='))
                .collect();
            let name = fields.get("Id")?;
            if !valid_name(name) || fields.get("LoadState") == Some(&"not-found") {
                return None;
            }
            let mut unit = base_unit(name);
            let get = |key, max| -> Option<String> {
                fields
                    .get(key)
                    .filter(|value| !value.is_empty())
                    .map(|value| truncate_utf8(value, max).to_owned())
            };
            if let Some(active) = get("ActiveState", 32) {
                unit.active = active;
            }
            if let Some(sub) = get("SubState", 32) {
                unit.sub = sub;
            }
            unit.description = get("Description", 512).unwrap_or_default();
            unit.exec_start = get("ExecStart", 1400).map(|value| redact::line(&value).to_owned());
            unit.working_directory = get("WorkingDirectory", 512);
            unit.fragment_path = get("FragmentPath", 512);
            unit.verified = true;
            Some(unit)
        })
        .collect()
}

fn systemctl(args: &[&str]) -> Result<String, CollectorError> {
    if !Path::new("/run/systemd/system").is_dir() || !Path::new("/usr/bin/systemctl").is_file() {
        return Err(CollectorError::UNAVAILABLE);
    }

    let mut child = Command::new("/usr/bin/systemctl")
        .args(["--system", "--no-pager", "--no-ask-password", "--full"])
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => CollectorError::UNAVAILABLE,
            io::ErrorKind::PermissionDenied => CollectorError::PERMISSION,
            _ => CollectorError::FAILED,
        })?;

    let result = read_child(&mut child);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let (status, stdout, stderr) = result?;
    if status.success() {
        return Ok(String::from_utf8_lossy(&stdout).into_owned());
    }

    let hint = String::from_utf8_lossy(&stderr).to_ascii_lowercase();
    if hint.contains("access denied") || hint.contains("permission denied") {
        return Err(CollectorError::PERMISSION);
    }
    if hint.contains("not been booted with systemd") || hint.contains("failed to connect to bus") {
        return Err(CollectorError::UNAVAILABLE);
    }
    if hint.contains("could not be found") || hint.contains("could not be loaded") {
        return Err(CollectorError::NOT_FOUND);
    }
    Err(CollectorError::FAILED)
}

fn read_child(child: &mut Child) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), CollectorError> {
    let mut stdout = child.stdout.take().ok_or(CollectorError::FAILED)?;
    let mut stderr = child.stderr.take().ok_or(CollectorError::FAILED)?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let entered = Instant::now();
    let mut output = Vec::new();
    let mut errors = Vec::new();
    loop {
        let eof_out = drain(&mut stdout, &mut output, MAX_STDOUT)?;
        let eof_err = drain(&mut stderr, &mut errors, MAX_STDERR)?;
        if let Some(status) = child.try_wait().map_err(|_| CollectorError::FAILED)?
            && eof_out
            && eof_err
        {
            return Ok((status, output, errors));
        }
        if entered.elapsed() >= COMMAND_TIMEOUT {
            return Err(CollectorError::TIMEOUT);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn nonblocking(stream: &impl AsRawFd) -> Result<(), CollectorError> {
    let fd = stream.as_raw_fd();
    // The child pipe is open for the life of stream and we preserve its existing flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(CollectorError::FAILED);
    }
    Ok(())
}

fn drain(stream: &mut impl Read, data: &mut Vec<u8>, limit: usize) -> Result<bool, CollectorError> {
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) if data.len() + count <= limit => data.extend_from_slice(&buffer[..count]),
            Ok(_) => return Err(CollectorError::LARGE),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(CollectorError::FAILED),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_unit_is_reported_with_source_and_redacted_exec() {
        let units = parse_properties(
            "Id=example.service\nLoadState=loaded\nActiveState=inactive\nSubState=dead\nExecStart=app --token=secret-value\nDescription=Example\n\nId=missing.service\nLoadState=not-found",
        );
        assert_eq!(units.len(), 1);
        let unit = &units[0];
        assert_eq!(unit.active, "inactive");
        assert_eq!(unit.sub, "dead");
        assert_eq!(unit.source, "systemd");
        assert!(unit.verified);
        assert!(!unit.exec_start.as_ref().unwrap().contains("secret-value"));
        assert!(!valid_name("../system.service"));
        assert!(!valid_name("*.service"));
    }
}
