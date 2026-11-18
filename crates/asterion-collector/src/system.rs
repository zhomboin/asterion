use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;

use crate::{CollectorError, mounts, observed_at, read_file};

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_INTERFACES: usize = 128;

#[derive(Default)]
pub(crate) struct Sampler {
    previous_cpu: Option<CpuTicks>,
    previous_network: Option<(Instant, HashMap<String, NetworkTicks>)>,
}

#[derive(Serialize)]
pub(crate) struct Sample {
    captured_at: u64,
    cpu_percent: Option<f64>,
    logical_cores: usize,
    load: [f64; 3],
    memory: Capacity,
    swap: Capacity,
    filesystems: Vec<mounts::FileSystem>,
    interfaces: Vec<Interface>,
}

#[derive(Serialize)]
struct Capacity {
    total_bytes: u64,
    available_bytes: u64,
    used_bytes: u64,
}

impl Capacity {
    fn new(total: u64, available: u64) -> Self {
        let available = available.min(total);
        Self {
            total_bytes: total,
            available_bytes: available,
            used_bytes: total - available,
        }
    }
}

#[derive(Serialize)]
struct Interface {
    name: String,
    rx_bytes: u64,
    tx_bytes: u64,
    rx_bytes_per_second: Option<f64>,
    tx_bytes_per_second: Option<f64>,
}

#[derive(Clone, Copy)]
pub(crate) struct CpuTicks {
    pub(crate) total: u64,
    idle: u64,
}

#[derive(Clone, Copy)]
struct NetworkTicks {
    rx: u64,
    tx: u64,
}

impl Sampler {
    pub(crate) fn sample(&mut self) -> Result<Sample, CollectorError> {
        let captured_at = observed_at();
        let now = Instant::now();
        let (ticks, logical_cores) = cpu_ticks(&read_proc("/proc/stat")?)?;
        let load = load_average(&read_proc("/proc/loadavg")?)?;
        let (memory, swap) = memory(&read_proc("/proc/meminfo")?)?;
        let network = network_ticks(&read_proc("/proc/net/dev")?);
        let filesystems = mounts::from_mountinfo(&read_proc("/proc/self/mountinfo")?);

        let cpu_percent = self
            .previous_cpu
            .replace(ticks)
            .and_then(|previous| cpu_percent(previous, ticks));
        let mut interfaces = interfaces(&network, self.previous_network.as_ref(), now);
        self.previous_network = Some((now, network));
        interfaces.sort_by(|first, second| first.name.cmp(&second.name));
        interfaces.truncate(MAX_INTERFACES);

        Ok(Sample {
            captured_at,
            cpu_percent,
            logical_cores,
            load,
            memory,
            swap,
            filesystems,
            interfaces,
        })
    }
}

pub(crate) fn read_proc(path: &str) -> Result<String, CollectorError> {
    read_file(path, FILE_LIMIT)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|_| CollectorError::FAILED)
}

pub(crate) fn cpu_ticks(stat: &str) -> Result<(CpuTicks, usize), CollectorError> {
    let mut lines = stat.lines();
    let aggregate = lines.next().ok_or(CollectorError::FAILED)?;
    let mut fields = aggregate.split_whitespace();
    if fields.next() != Some("cpu") {
        return Err(CollectorError::FAILED);
    }
    let ticks: Vec<u64> = fields
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| CollectorError::FAILED)?;
    if ticks.len() < 4 {
        return Err(CollectorError::FAILED);
    }
    let cpu = CpuTicks {
        total: ticks.iter().sum(),
        idle: ticks[3].saturating_add(ticks.get(4).copied().unwrap_or_default()),
    };
    let logical_cores = lines
        .take_while(|line| {
            line.strip_prefix("cpu")
                .and_then(|suffix| suffix.as_bytes().first())
                .is_some_and(u8::is_ascii_digit)
        })
        .count();
    Ok((cpu, logical_cores.max(1)))
}

fn cpu_percent(before: CpuTicks, after: CpuTicks) -> Option<f64> {
    let elapsed = after.total.checked_sub(before.total)?;
    let idle = after.idle.checked_sub(before.idle)?;
    if elapsed == 0 || idle > elapsed {
        return None;
    }
    Some((elapsed - idle) as f64 * 100.0 / elapsed as f64)
}

fn load_average(contents: &str) -> Result<[f64; 3], CollectorError> {
    let items = contents
        .split_whitespace()
        .take(3)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CollectorError::FAILED)?;
    if items.iter().any(|value| !value.is_finite() || *value < 0.0) {
        return Err(CollectorError::FAILED);
    }
    items.try_into().map_err(|_| CollectorError::FAILED)
}

fn memory(contents: &str) -> Result<(Capacity, Capacity), CollectorError> {
    let values: HashMap<_, _> = contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let kib = value.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key, kib.saturating_mul(1024)))
        })
        .collect();
    let get = |name| values.get(name).copied().ok_or(CollectorError::FAILED);

    Ok((
        Capacity::new(get("MemTotal")?, get("MemAvailable")?),
        Capacity::new(get("SwapTotal")?, get("SwapFree")?),
    ))
}

fn network_ticks(contents: &str) -> HashMap<String, NetworkTicks> {
    contents
        .lines()
        .filter_map(|line| {
            let (name, values) = line.rsplit_once(':')?;
            let name = name.trim();
            if name.is_empty() || name.len() > 64 {
                return None;
            }
            let mut fields = values.split_whitespace();
            let rx = fields.next()?.parse().ok()?;
            let tx = fields.nth(7)?.parse().ok()?;
            Some((name.to_owned(), NetworkTicks { rx, tx }))
        })
        .collect()
}

fn interfaces(
    current: &HashMap<String, NetworkTicks>,
    previous: Option<&(Instant, HashMap<String, NetworkTicks>)>,
    now: Instant,
) -> Vec<Interface> {
    current
        .iter()
        .map(|(name, ticks)| {
            let elapsed = previous.map(|(time, _)| now.duration_since(*time).as_secs_f64());
            let before = previous.and_then(|(_, counters)| counters.get(name));
            Interface {
                name: name.clone(),
                rx_bytes: ticks.rx,
                tx_bytes: ticks.tx,
                rx_bytes_per_second: rate(before.map(|value| value.rx), ticks.rx, elapsed),
                tx_bytes_per_second: rate(before.map(|value| value.tx), ticks.tx, elapsed),
            }
        })
        .collect()
}

fn rate(previous: Option<u64>, current: u64, seconds: Option<f64>) -> Option<f64> {
    let delta = current.checked_sub(previous?)?;
    let seconds = seconds.filter(|value| *value > 0.0)?;
    Some(delta as f64 / seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_excludes_guest_ticks_and_missing_or_reset_counters() {
        let (before, cores) =
            cpu_ticks("cpu 14 0 11 40 0 0 0 0 42 1\ncpu0 99 0 0 0\ncpu1 99 0 0 0\nbtime 2")
                .unwrap();
        let (after, _) = cpu_ticks("cpu 24 0 11 50 0 0 0 0 100 7").unwrap();
        assert_eq!(cores, 2);
        assert_eq!(cpu_percent(before, after), Some(50.0));
        assert_eq!(cpu_percent(after, before), None);
        assert_eq!(cpu_percent(after, after), None);
    }

    #[test]
    fn network_rate_is_missing_until_adjacent_valid_counters_exist() {
        assert_eq!(rate(Some(5), 5, Some(0.5)), Some(0.0));
        assert_eq!(rate(Some(5), 15, Some(0.5)), Some(20.0));
        assert_eq!(rate(Some(5), 4, Some(0.5)), None);
        assert_eq!(rate(None, 4, Some(0.5)), None);
    }
}
