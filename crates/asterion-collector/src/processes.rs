use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::process_runtime::{Runtime, runtime_from_cgroups};
use crate::system::{CpuTicks, cpu_ticks, read_proc};
use crate::{CollectorError, read_file, redact, truncate_utf8};

const MAX_PROCESSES_SCAN: usize = 8192;
const MAX_LIST_SIZE: usize = 256;

#[derive(Default)]
pub(crate) struct Sampler {
    previous: Option<Snapshot>,
}

struct Snapshot {
    system_ticks: u64,
    processes: HashMap<u32, (u64, u64)>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListArgs {
    q: Option<String>,
    names: Option<Vec<String>>,
    #[serde(default)]
    sort: Sort,
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Sort {
    #[default]
    Cpu,
    Memory,
    Name,
    Pid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetailArgs {
    pid: u32,
}

#[derive(Clone, Serialize)]
struct Process {
    pid: u32,
    name: String,
    state: char,
    cpu_percent: Option<f64>,
    memory_bytes: Option<u64>,
    started_at: Option<u64>,
    start_ticks: u64,
}

#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    process: Process,
    command_line: Option<Vec<String>>,
    executable: Option<String>,
    parent_pid: Option<u32>,
    user: Option<String>,
    runtime: Option<Runtime>,
}

struct ProcStat {
    name: String,
    state: char,
    parent_pid: u32,
    time_ticks: u64,
    start_ticks: u64,
    rss_pages: Option<u64>,
}

struct HostStats {
    cpu: CpuTicks,
    cores: usize,
    ticks_per_second: Option<u64>,
    page_size: Option<u64>,
    boot_time: Option<u64>,
    uptime: Option<f64>,
}

impl HostStats {
    fn read() -> Result<Self, CollectorError> {
        let stat = read_proc("/proc/stat")?;
        let (cpu, cores) = cpu_ticks(&stat)?;
        Ok(Self {
            cpu,
            cores,
            ticks_per_second: sysconf(libc::_SC_CLK_TCK),
            page_size: sysconf(libc::_SC_PAGESIZE),
            boot_time: stat
                .lines()
                .find_map(|line| line.strip_prefix("btime ")?.parse().ok()),
            uptime: read_proc("/proc/uptime")
                .ok()
                .and_then(|value| value.split_whitespace().next()?.parse().ok()),
        })
    }

    fn lifetime_cpu(&self, stat: &ProcStat) -> Option<f64> {
        let ticks = self.ticks_per_second? as f64;
        let elapsed = self.uptime? - stat.start_ticks as f64 / ticks;
        if elapsed <= 0.0 {
            return None;
        }
        Some((stat.time_ticks as f64 * 100.0 / ticks / elapsed).min(self.cores as f64 * 100.0))
    }

    fn process(&self, pid: u32, stat: &ProcStat, cpu: Option<f64>) -> Process {
        Process {
            pid,
            name: stat.name.clone(),
            state: stat.state,
            cpu_percent: cpu.or_else(|| self.lifetime_cpu(stat)),
            memory_bytes: stat
                .rss_pages
                .zip(self.page_size)
                .and_then(|(pages, page_size)| pages.checked_mul(page_size)),
            started_at: self
                .boot_time
                .zip(self.ticks_per_second)
                .and_then(|(boot, ticks)| boot.checked_add(stat.start_ticks / ticks)),
            start_ticks: stat.start_ticks,
        }
    }
}

impl Sampler {
    pub(crate) fn list(&mut self, args: ListArgs) -> Result<Value, CollectorError> {
        let limit = args.limit.unwrap_or(MAX_LIST_SIZE);
        if !(1..=MAX_LIST_SIZE).contains(&limit)
            || args.offset > MAX_PROCESSES_SCAN
            || (args.names.is_some() && (args.offset != 0 || limit > 13))
            || args.names.as_ref().is_some_and(|names| {
                names.is_empty()
                    || names.len() > 12
                    || names.iter().any(|name| {
                        name.is_empty() || name.len() > 256 || name.chars().any(char::is_control)
                    })
            })
            || args.q.as_ref().is_some_and(|value| value.len() > 256)
        {
            return Err(CollectorError::INVALID);
        }
        let stats = HostStats::read()?;
        let mut snapshot = Snapshot {
            system_ticks: stats.cpu.total,
            processes: HashMap::new(),
        };
        let query = args.q.unwrap_or_default().trim().to_lowercase();
        let mut processes = Vec::new();
        let entries = fs::read_dir("/proc").map_err(|_| CollectorError::FAILED)?;
        for entry in entries.flatten().take(MAX_PROCESSES_SCAN) {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = read_stat(pid) else { continue };
            snapshot
                .processes
                .insert(pid, (stat.start_ticks, stat.time_ticks));
            if !query.is_empty()
                && !pid.to_string().contains(&query)
                && !stat.name.to_lowercase().contains(&query)
            {
                continue;
            }
            if args
                .names
                .as_ref()
                .is_some_and(|names| !names.contains(&stat.name))
            {
                continue;
            }
            let cpu = self.previous.as_ref().and_then(|before| {
                let &(start, ticks) = before.processes.get(&pid)?;
                let system_delta = stats.cpu.total.checked_sub(before.system_ticks)?;
                if start != stat.start_ticks || system_delta == 0 {
                    return None;
                }
                let delta = stat.time_ticks.checked_sub(ticks)?;
                let maximum = stats.cores as f64 * 100.0;
                Some((delta as f64 / system_delta as f64 * maximum).min(maximum))
            });
            processes.push(stats.process(pid, &stat, cpu));
        }
        self.previous = Some(snapshot);
        processes.sort_by(|one, two| compare(one, two, args.sort));
        if args.names.is_some() {
            let mut counts = HashMap::new();
            processes.retain(|process| {
                let count = counts.entry(process.name.clone()).or_insert(0);
                *count += 1;
                *count <= limit
            });
            return serde_json::to_value(processes).map_err(|_| CollectorError::FAILED);
        }
        let page: Vec<_> = processes
            .into_iter()
            .skip(args.offset)
            .take(limit)
            .collect();
        serde_json::to_value(page).map_err(|_| CollectorError::FAILED)
    }

    pub(crate) fn detail(&self, args: DetailArgs) -> Result<Value, CollectorError> {
        let pid = args.pid;
        let stat = read_stat(pid)?;
        let stats = HostStats::read()?;
        let path = format!("/proc/{pid}");
        let command_line = read_file(format!("{path}/cmdline"), 4096)
            .ok()
            .map(|bytes| command_arguments(&bytes));
        let executable = fs::read_link(format!("{path}/exe"))
            .ok()
            .map(|value| truncate_utf8(&value.to_string_lossy(), 2048).to_owned());
        let user = read_file(format!("{path}/status"), 16 * 1024)
            .ok()
            .and_then(|contents| find_user(&String::from_utf8_lossy(&contents)));
        let runtime = read_file(format!("{path}/cgroup"), 16 * 1024)
            .ok()
            .and_then(|contents| runtime_from_cgroups(std::str::from_utf8(&contents).ok()?));
        if stat.start_ticks != read_stat(pid)?.start_ticks {
            return Err(CollectorError::NOT_FOUND);
        }
        serde_json::to_value(Detail {
            process: stats.process(pid, &stat, None),
            command_line,
            executable,
            parent_pid: Some(stat.parent_pid),
            user,
            runtime,
        })
        .map_err(|_| CollectorError::FAILED)
    }
}

fn command_arguments(bytes: &[u8]) -> Vec<String> {
    let Some(last_complete) = bytes.iter().rposition(|byte| *byte == 0) else {
        return Vec::new();
    };
    redact::arguments(
        bytes[..last_complete]
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty())
            .take(32)
            .map(|field| String::from_utf8_lossy(field).into_owned()),
    )
    .into_iter()
    .map(|field| truncate_utf8(&field, 512).to_owned())
    .collect()
}

fn compare(one: &Process, two: &Process, sort: Sort) -> Ordering {
    match sort {
        Sort::Cpu => two
            .cpu_percent
            .partial_cmp(&one.cpu_percent)
            .unwrap_or(Ordering::Equal),
        Sort::Memory => two.memory_bytes.cmp(&one.memory_bytes),
        Sort::Name => one.name.cmp(&two.name),
        Sort::Pid => one.pid.cmp(&two.pid),
    }
    .then_with(|| one.pid.cmp(&two.pid))
}

pub fn process_start_ticks(pid: u32) -> Result<u64, CollectorError> {
    read_stat(pid).map(|stat| stat.start_ticks)
}

fn read_stat(pid: u32) -> Result<ProcStat, CollectorError> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(CollectorError::INVALID);
    }
    let bytes =
        read_file(format!("/proc/{pid}/stat"), 4096).map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => CollectorError::PERMISSION,
            std::io::ErrorKind::NotFound => CollectorError::NOT_FOUND,
            _ => CollectorError::FAILED,
        })?;
    parse_stat(&String::from_utf8_lossy(&bytes)).ok_or(CollectorError::FAILED)
}

fn parse_stat(value: &str) -> Option<ProcStat> {
    let (_, rest) = value.split_once('(')?;
    let (name, fields) = rest.rsplit_once(") ")?;
    let mut fields = fields.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let parent_pid = fields.next()?.parse().ok()?;
    let user_ticks = fields.nth(9)?.parse::<u64>().ok()?;
    let system_ticks = fields.next()?.parse::<u64>().ok()?;
    let start_ticks = fields.nth(6)?.parse().ok()?;
    let rss_pages = fields.nth(1)?.parse().ok();
    Some(ProcStat {
        name: truncate_utf8(name, 128)
            .chars()
            .map(|ch| if ch.is_control() { '\u{fffd}' } else { ch })
            .collect(),
        state,
        parent_pid,
        time_ticks: user_ticks.checked_add(system_ticks)?,
        start_ticks,
        rss_pages,
    })
}

fn find_user(status: &str) -> Option<String> {
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u32>()
        .ok()?;
    let passwd = read_file("/etc/passwd", 1024 * 1024).ok();
    let owner = passwd
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                let id = fields.nth(1)?.parse::<u32>().ok()?;
                (id == uid).then(|| truncate_utf8(name, 128).to_owned())
            })
        });
    Some(owner.unwrap_or_else(|| uid.to_string()))
}

fn sysconf(name: libc::c_int) -> Option<u64> {
    let result = unsafe { libc::sysconf(name) };
    (result > 0).then_some(result as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_parser_accepts_parentheses_in_executable_names() {
        let raw = "1 (contains ) parens) S 2 0 0 0 0 0 0 0 0 0 123 45 0 0 0 0 2 0 987 1048576 43";
        let stat = parse_stat(raw).unwrap();
        assert_eq!(stat.name, "contains ) parens");
        assert_eq!(stat.parent_pid, 2);
        assert_eq!(stat.time_ticks, 168);
        assert_eq!(stat.start_ticks, 987);
        assert_eq!(stat.rss_pages, Some(43));
    }

    #[test]
    fn command_arguments_redact_before_truncation_and_hide_incomplete_fields() {
        let bytes = format!(
            "tool\0https://user:{}@example.test\0regular\0partial",
            "p".repeat(520)
        );
        let args = command_arguments(bytes.as_bytes());
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "tool");
        assert!(args[1].contains("隐藏"));
        assert_eq!(args[2], "regular");
        assert!(command_arguments(b"incomplete").is_empty());
    }

    #[test]
    fn self_process_reports_real_identity_and_memory() {
        let pid = std::process::id();
        let result = Sampler::default().detail(DetailArgs { pid }).unwrap();
        assert_eq!(result["pid"], pid);
        assert!(result["start_ticks"].as_u64().unwrap() > 0);
        assert!(result["memory_bytes"].as_u64().unwrap() > 0);
    }
}
