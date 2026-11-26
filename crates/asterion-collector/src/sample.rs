use anyhow::{Context, Result, ensure};
use asterion_protocol::collector::MAX_SAMPLE_BYTES;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, time::Instant};

struct Counters {
    time: Instant,
    cpu: u64,
    idle: u64,
    net: BTreeMap<String, (u64, u64)>,
}

#[derive(Default)]
pub struct Sampler {
    previous: Option<Counters>,
}

impl Sampler {
    pub fn sample(&mut self, captured_at: i64) -> Result<Value> {
        let stat = fs::read_to_string("/proc/stat")?;
        let ticks: Vec<u64> = stat
            .lines()
            .next()
            .context("缺少 CPU 计数器")?
            .split_whitespace()
            .skip(1)
            .take(8)
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        ensure!(ticks.len() >= 5, "CPU 计数器不完整");
        let current = Counters {
            time: Instant::now(),
            cpu: ticks.iter().sum(),
            idle: ticks[3] + ticks[4],
            net: network(&String::from_utf8_lossy(&fs::read("/proc/net/dev")?)),
        };
        let cpu = self.previous.as_ref().and_then(|old| {
            let total = current.cpu.checked_sub(old.cpu)?;
            let idle = current.idle.checked_sub(old.idle)?;
            (total > 0).then(|| 100.0 * total.saturating_sub(idle) as f64 / total as f64)
        });
        let mut interfaces = Vec::new();
        for (name, &(rx, tx)) in &current.net {
            let rates = self
                .previous
                .as_ref()
                .and_then(|old| {
                    let (r, t) = old.net.get(name)?;
                    let dt = current.time.duration_since(old.time).as_secs_f64();
                    (dt > 0.0).then(|| {
                        (
                            rx.checked_sub(*r).map(|n| n as f64 / dt),
                            tx.checked_sub(*t).map(|n| n as f64 / dt),
                        )
                    })
                })
                .unwrap_or_default();
            interfaces.push(json!({"name":name,"rx_bytes":rx,"tx_bytes":tx,
                "rx_bytes_per_second":rates.0,"tx_bytes_per_second":rates.1}));
        }
        self.previous = Some(current);
        let info = fs::read_to_string("/proc/meminfo")?;
        let mem = |key: &str| -> Result<u64> {
            info.lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|s| s.split_whitespace().next())
                .context("缺少内存计数器")?
                .parse::<u64>()?
                .checked_mul(1024)
                .context("内存计数器溢出")
        };
        let capacity = |total: u64, free: u64| {
            let available = free.min(total);
            json!({"total_bytes":total,"available_bytes":available,"used_bytes":total-available})
        };
        let load = fs::read_to_string("/proc/loadavg")?
            .split_whitespace()
            .take(3)
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>()?;
        let cores = stat
            .lines()
            .filter_map(|line| line.strip_prefix("cpu"))
            .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            .count();
        let mut sample = json!({"captured_at":captured_at,"logical_cores":cores, "cpu_percent":cpu,
            "load":load, "memory":capacity(mem("MemTotal:")?,mem("MemAvailable:")?),
            "swap":capacity(mem("SwapTotal:")?,mem("SwapFree:")?), "interfaces":interfaces,"filesystems":[]});
        let mut remaining = MAX_SAMPLE_BYTES.saturating_sub(sample.to_string().len());
        for fs in crate::mounts::filesystems()? {
            let size = fs.to_string().len() + 1;
            if size <= remaining {
                remaining -= size;
                sample["filesystems"].as_array_mut().unwrap().push(fs);
            }
        }
        Ok(sample)
    }
}

fn network(data: &str) -> BTreeMap<String, (u64, u64)> {
    data.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, data) = line.rsplit_once(':')?;
            let name = name.trim();
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return None;
            }
            let mut fields = data.split_whitespace();
            let rx = fields.next()?.parse().ok()?;
            let tx = fields.nth(7)?.parse().ok()?;
            Some((name.to_owned(), (rx, tx)))
        })
        .take(128)
        .collect()
}
