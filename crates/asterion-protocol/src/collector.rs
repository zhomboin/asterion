use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_SAMPLE_BYTES: usize = 48 * 1024;
pub const BATCH_BYTES: usize = 192 * 1024;
pub const BATCH_ROWS: usize = 256;

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Cursor {
    pub stream: String,
    pub seq: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Record {
    pub seq: i64,
    pub sample: Value,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Batch {
    pub stream: String,
    pub records: Vec<Record>,
    pub newest_seq: i64,
    pub newest_at: Option<i64>,
    pub interval_secs: u64,
    pub arch: String,
}

pub fn valid_sample(value: &Value) -> bool {
    metrics_sample(value).is_some()
}

pub fn metrics_sample(sample: &Value) -> Option<String> {
    let map = sample.as_object()?;
    if map.len() != 8
        || !map
            .get("captured_at")
            .and_then(Value::as_i64)
            .is_some_and(|time| time >= 0)
        || !map
            .get("logical_cores")
            .and_then(Value::as_u64)
            .is_some_and(|cores| cores > 0 && cores <= 262_144)
        || !map.get("cpu_percent").is_some_and(percent)
        || !map
            .get("load")
            .and_then(Value::as_array)
            .is_some_and(|load| load.len() == 3 && load.iter().all(nonnegative_number))
        || !map.get("memory").is_some_and(memory)
        || !map.get("swap").is_some_and(memory)
        || !map
            .get("filesystems")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() <= 256 && items.iter().all(filesystem))
        || !map
            .get("interfaces")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() <= 256 && items.iter().all(interface))
    {
        return None;
    }
    let encoded = sample.to_string();
    (encoded.len() <= MAX_SAMPLE_BYTES).then_some(encoded)
}

fn percent(value: &Value) -> bool {
    value.is_null()
        || value
            .as_f64()
            .is_some_and(|number| (0.0..=100.0).contains(&number))
}

fn nonnegative_number(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number >= 0.0 && number.is_finite())
}

fn memory(value: &Value) -> bool {
    let Some(total) = value.get("total_bytes").and_then(Value::as_u64) else {
        return false;
    };
    let Some(available) = value.get("available_bytes").and_then(Value::as_u64) else {
        return false;
    };
    value.as_object().is_some_and(|map| map.len() == 3)
        && total
            .checked_sub(available)
            .is_some_and(|used| value.get("used_bytes").and_then(Value::as_u64) == Some(used))
}

fn filesystem(value: &Value) -> bool {
    let total = value.get("total_bytes").and_then(Value::as_u64);
    let available = value.get("available_bytes").and_then(Value::as_u64);
    value.as_object().is_some_and(|map| map.len() == 4)
        && bounded_string(value.get("mountpoint"), 4096)
        && bounded_string(value.get("fs_type"), 100)
        && matches!((total, available), (Some(total), Some(available)) if available <= total)
}

fn interface(value: &Value) -> bool {
    value.as_object().is_some_and(|map| map.len() == 5)
        && bounded_string(value.get("name"), 256)
        && ["rx_bytes", "tx_bytes"]
            .iter()
            .all(|name| value.get(name).and_then(Value::as_u64).is_some())
        && ["rx_bytes_per_second", "tx_bytes_per_second"]
            .iter()
            .all(|name| {
                value
                    .get(name)
                    .is_some_and(|number| number.is_null() || nonnegative_number(number))
            })
}

fn bounded_string(value: Option<&Value>, limit: usize) -> bool {
    value.and_then(Value::as_str).is_some_and(|value| {
        !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
    })
}
