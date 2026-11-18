use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{CollectorError, MAX_QUERY_BYTES, observed_at, redact, truncate_utf8};

use super::{Request, transport};

const MAX_CONTAINERS: usize = 96;
const MAX_LIST_BYTES: usize = 4 * 1024 * 1024;
const MAX_DETAIL_BYTES: usize = 512 * 1024;
const MAX_LOG_BYTES: usize = 32 * 1024;

pub(super) fn query(
    socket: &Path,
    request: Request,
    deadline: Instant,
) -> Result<Value, CollectorError> {
    let inventory = matches!(request, Request::Docker);
    if let Some(status) = endpoint_state(socket) {
        return if inventory {
            Ok(inventory_status(status))
        } else {
            Err(if status == "permission_denied" {
                CollectorError::PERMISSION
            } else {
                CollectorError::UNAVAILABLE
            })
        };
    }
    let engine = Engine { socket, deadline };
    if inventory {
        return match engine.inventory() {
            Err(CollectorError {
                code: "unavailable" | "timeout",
                ..
            }) => Ok(inventory_status("unreachable")),
            Err(CollectorError {
                code: "permission_denied",
                ..
            }) => Ok(inventory_status("permission_denied")),
            result => result,
        };
    }
    match request {
        Request::Docker => unreachable!(),
        Request::DockerStats(container) => engine.stats(&container.id),
        Request::DockerLogs(logs) => engine.logs(&logs.id, logs.lines.unwrap_or(100)),
    }
}

struct Engine<'a> {
    socket: &'a Path,
    deadline: Instant,
}

impl Engine<'_> {
    fn get_json(&self, path: &str, limit: usize) -> Result<Value, CollectorError> {
        let result = transport::get(self.socket, path, limit, false, self.deadline)?;
        check_http_status(result.status)?;
        serde_json::from_slice(&result.content).map_err(|_| CollectorError::FAILED)
    }

    fn inventory(&self) -> Result<Value, CollectorError> {
        let version = self.get_json("/version", 64 * 1024)?;
        let info = self.get_json("/info", MAX_DETAIL_BYTES)?;
        let list = self.get_json("/containers/json?all=1", MAX_LIST_BYTES)?;
        let list = list.as_array().ok_or(CollectorError::FAILED)?;
        let mut containers = Vec::new();
        let mut total = 0;
        let mut truncated = list.len() > MAX_CONTAINERS;
        for listed in list.iter().take(MAX_CONTAINERS) {
            let Some(id) = listed["Id"].as_str().filter(|id| valid_id(id)) else {
                continue;
            };
            if self
                .deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                < 350
            {
                truncated = true;
                break;
            }
            let detail = match self.get_json(&format!("/containers/{id}/json"), MAX_DETAIL_BYTES) {
                Ok(detail) => Some(detail),
                Err(CollectorError {
                    code: "timeout", ..
                }) => {
                    truncated = true;
                    None
                }
                Err(_) => None,
            };
            let row = container_row(listed, detail.as_ref());
            let size = serde_json::to_vec(&row)
                .map_err(|_| CollectorError::FAILED)?
                .len();
            if total + size + 1 > MAX_QUERY_BYTES - 2048 {
                truncated = true;
                break;
            }
            containers.push(row);
            total += size + 1;
        }

        let os = clipped(&info["OperatingSystem"], 192);
        Ok(json!({
            "status": "available",
            "engine_id": clipped(&info["ID"], 128),
            "engine_version": clipped(&version["Version"], 128),
            "engine_kind": if os.as_deref().is_some_and(|os| os.to_ascii_lowercase().contains("docker desktop")) {
                "desktop"
            } else {
                "native"
            },
            "endpoint_fingerprint": fingerprint(self.socket),
            "containers": containers,
            "truncated": truncated,
        }))
    }

    fn stats(&self, id: &str) -> Result<Value, CollectorError> {
        let value = self.get_json(
            &format!("/containers/{id}/stats?stream=false"),
            MAX_DETAIL_BYTES,
        )?;
        let (cpu, memory, memory_limit, rx, tx) = stats_for(&value);
        Ok(json!({
            "cpu_percent":cpu,
            "memory_bytes":memory,
            "memory_limit_bytes":memory_limit,
            "rx_bytes":rx,
            "tx_bytes":tx,
        }))
    }

    fn logs(&self, id: &str, lines: u16) -> Result<Value, CollectorError> {
        let detail = self.get_json(&format!("/containers/{id}/json"), MAX_DETAIL_BYTES)?;
        let plain = detail["Config"]["Tty"].as_bool().unwrap_or(false);
        let since = observed_at().saturating_sub(24 * 60 * 60);
        let output = transport::get(
            self.socket,
            &format!(
                "/containers/{id}/logs?stdout=1&stderr=1&timestamps=1&tail={lines}&since={since}"
            ),
            MAX_LOG_BYTES * 2,
            true,
            self.deadline,
        )?;
        check_http_status(output.status)?;
        let (log, truncated_frames) = if plain {
            (output.content, false)
        } else {
            read_log_frames(&output.content)?
        };
        let (text, truncated) = safe_logs(&log, output.truncated || truncated_frames);
        Ok(json!({"text":text,"truncated":truncated}))
    }
}

fn check_http_status(status: u16) -> Result<(), CollectorError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(CollectorError::PERMISSION),
        404 => Err(CollectorError::NOT_FOUND),
        500..=599 => Err(CollectorError::new(
            "docker_error",
            "Docker 引擎未能处理读取请求",
        )),
        _ => Err(CollectorError::FAILED),
    }
}

fn endpoint_state(socket: &Path) -> Option<&'static str> {
    match std::fs::metadata(socket) {
        Ok(metadata) if metadata.file_type().is_socket() => None,
        Ok(_) => Some("unreachable"),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            Some("permission_denied")
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && ![
                    Path::new("/usr/bin/docker"),
                    Path::new("/usr/local/bin/docker"),
                ]
                .iter()
                .any(|path| path.is_file()) =>
        {
            Some("not_installed")
        }
        Err(_) => Some("unreachable"),
    }
}

pub(super) fn inventory_status(status: &str) -> Value {
    json!({
        "status": status,
        "engine_id": null,
        "engine_version": null,
        "engine_kind": null,
        "endpoint_fingerprint": null,
        "containers": [],
        "truncated": false,
    })
}

fn fingerprint(socket: &Path) -> String {
    format!("{:x}", Sha256::digest(socket.as_os_str().as_bytes()))
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn clipped(value: &Value, max: usize) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| truncate_utf8(value, max).to_owned())
}

fn container_row(listed: &Value, detail: Option<&Value>) -> Value {
    let null = Value::Null;
    let detail = detail.unwrap_or(&null);
    let state = &detail["State"];
    let labels = &listed["Labels"];
    let name = listed["Names"]
        .get(0)
        .and_then(Value::as_str)
        .or_else(|| detail["Name"].as_str())
        .unwrap_or_default()
        .trim_start_matches('/');
    let mounts = listed["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(|mount| {
            Some(json!({
                "source": clipped(&mount["Source"], 400)?,
                "destination": clipped(&mount["Destination"], 400)?,
                "mode": clipped(&mount["Mode"], 24).unwrap_or_else(|| {
                    if mount["RW"].as_bool() == Some(true) { "rw" } else { "ro" }.into()
                }),
            }))
        })
        .collect::<Vec<_>>();
    let ports = listed["Ports"].as_array().into_iter().flatten().take(32).filter_map(|port| {
        let private_port = u16::try_from(port["PrivatePort"].as_u64()?).ok()?;
        Some(json!({
            "ip": clipped(&port["IP"], 64),
            "private_port": private_port,
            "public_port": port["PublicPort"].as_u64().and_then(|value| u16::try_from(value).ok()),
            "protocol": clipped(&port["Type"], 16).unwrap_or_default(),
        }))
    }).collect::<Vec<_>>();
    json!({
        "id": listed["Id"],
        "name": truncate_utf8(name, 256),
        "image": clipped(&listed["Image"], 256).unwrap_or_default(),
        "state": clipped(&listed["State"], 40).unwrap_or_else(|| "unknown".into()),
        "status": clipped(&listed["Status"], 192).unwrap_or_default(),
        "health": clipped(&state["Health"]["Status"], 40),
        "started_at": state["StartedAt"].as_str().and_then(unix_timestamp),
        "restart_count": detail["RestartCount"].as_u64(),
        "compose_project": clipped(&labels["com.docker.compose.project"], 128),
        "compose_service": clipped(&labels["com.docker.compose.service"], 128),
        "mounts": mounts,
        "ports": ports,
    })
}

type Stats = (
    Option<f64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
);

fn stats_for(value: &Value) -> Stats {
    let current = &value["cpu_stats"];
    let previous = &value["precpu_stats"];
    let cpu = (|| {
        let used = current["cpu_usage"]["total_usage"]
            .as_u64()?
            .checked_sub(previous["cpu_usage"]["total_usage"].as_u64()?)?;
        let system = current["system_cpu_usage"]
            .as_u64()?
            .checked_sub(previous["system_cpu_usage"].as_u64()?)?;
        let cores = current["online_cpus"].as_u64().or_else(|| {
            current["cpu_usage"]["percpu_usage"]
                .as_array()
                .map(|cores| cores.len() as u64)
        })?;
        (system > 0 && cores > 0).then_some(used as f64 / system as f64 * cores as f64 * 100.0)
    })();

    let memory = &value["memory_stats"];
    let cached = memory["stats"]["total_inactive_file"]
        .as_u64()
        .or_else(|| memory["stats"]["inactive_file"].as_u64())
        .unwrap_or_default();
    let memory_bytes = memory["usage"]
        .as_u64()
        .map(|usage| usage.saturating_sub(cached));
    let memory_limit = memory["limit"].as_u64();
    let network = value["networks"].as_object();
    let sum = |key| {
        network?.values().try_fold(0_u64, |total, iface| {
            total.checked_add(iface[key].as_u64()?)
        })
    };
    (
        cpu,
        memory_bytes,
        memory_limit,
        sum("rx_bytes"),
        sum("tx_bytes"),
    )
}

fn read_log_frames(mut input: &[u8]) -> Result<(Vec<u8>, bool), CollectorError> {
    let mut output = Vec::with_capacity(input.len());
    while input.len() >= 8 {
        if !matches!(input[0], 1 | 2) || input[1..4] != [0, 0, 0] {
            return Err(CollectorError::FAILED);
        }
        let size = u32::from_be_bytes(input[4..8].try_into().expect("checked header")) as usize;
        input = &input[8..];
        if size > input.len() {
            return Ok((output, true));
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size..];
    }
    Ok((output, !input.is_empty()))
}

fn safe_logs(log: &[u8], mut truncated: bool) -> (String, bool) {
    let decoded = String::from_utf8_lossy(log);
    let mut output = String::new();
    for value in decoded.split_inclusive('\n') {
        // An incomplete final line can omit the marker that identifies the secret.
        if truncated && !value.ends_with('\n') {
            break;
        }
        let line: String = value
            .chars()
            .filter(|ch| !ch.is_control() || matches!(*ch, '\n' | '\t'))
            .collect();
        let content = redact::line(line.trim_end_matches('\n'));
        if output.len() + content.len() + 1 > MAX_LOG_BYTES {
            truncated = true;
            break;
        }
        output.push_str(content);
        if value.ends_with('\n') {
            output.push('\n');
        }
    }
    (output, truncated)
}

fn unix_timestamp(value: &str) -> Option<u64> {
    let (date, time) = value.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let year = date.next()?.ok()?;
    let month = date.next()?.ok()?;
    let day = date.next()?.ok()?;
    let mut time = time.split('.').next()?.split(':').map(str::parse::<i64>);
    let hour = time.next()?.ok()?;
    let minute = time.next()?.ok()?;
    let second = time.next()?.ok()?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    // Convert the UTC Gregorian calendar date using a March-based year.
    let year = year - i64::from(month <= 2);
    let era = year / 400;
    let in_era = year % 400;
    let month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let days = era * 146097 + in_era * 365 + in_era / 4 - in_era / 100 + day_of_year - 719468;
    u64::try_from(days * 86400 + hour * 3600 + minute * 60 + second).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_inventory_omits_any_environment_values() {
        let listed = json!({
            "Id":"a".repeat(64),"Names":["/sample"],"Image":"image","State":"running",
            "Labels": {"com.docker.compose.project":"demo","private":"sensitive"},
            "Mounts":[{"Source":"/tmp","Destination":"/data","RW":false}]
        });
        let inspect = json!({
            "Config":{"Env":["TOKEN=sensitive"]},"RestartCount":3,
            "State":{"StartedAt":"1970-01-02T01:02:03.012Z","Health":{"Status":"healthy"}}
        });
        let row = container_row(&listed, Some(&inspect));
        assert_eq!(row["name"], "sample");
        assert_eq!(row["compose_project"], "demo");
        assert_eq!(row["started_at"], 90123);
        assert_eq!(row["restart_count"], 3);
        assert_eq!(row["health"], "healthy");
        assert!(!row.to_string().contains("sensitive"));
    }

    #[test]
    fn multiplexed_logs_are_redacted_and_partial_lines_dropped() {
        let body = b"normal\npassword=example\nlast";
        let mut frame = vec![1, 0, 0, 0];
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(body);
        let (decoded, partial) = read_log_frames(&frame).unwrap();
        assert!(!partial);
        let (text, truncated) = safe_logs(&decoded, true);
        assert!(truncated);
        assert!(text.contains("normal"));
        assert!(!text.contains("example"));
        assert!(!text.contains("last"));
    }
}
