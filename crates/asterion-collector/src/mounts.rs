use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::HashSet, ffi::CString, fs, mem::MaybeUninit};

pub fn filesystems() -> Result<Vec<Value>> {
    let mounts = fs::read("/proc/self/mounts")?;
    let list = String::from_utf8_lossy(&mounts);
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for line in list.lines() {
        let mut fields = line.split_whitespace().skip(1);
        let (Some(path), Some(kind)) = (fields.next(), fields.next()) else {
            continue;
        };
        let path = path.replace(r"\040", " ").replace(r"\134", "\\");
        if path.len() > 4096 || path.chars().any(char::is_control) || kind.len() > 100 {
            continue;
        }
        if path != "/"
            && (matches!(
                kind,
                "proc"
                    | "sysfs"
                    | "tmpfs"
                    | "devtmpfs"
                    | "devpts"
                    | "cgroup"
                    | "cgroup2"
                    | "squashfs"
                    | "ramfs"
                    | "autofs"
                    | "mqueue"
                    | "debugfs"
                    | "tracefs"
            ) || ["/sys/", "/proc/", "/dev/"]
                .iter()
                .any(|p| path.starts_with(p)))
        {
            continue;
        }
        if path.contains(r"\011") || path.contains(r"\012") || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(cpath) = CString::new(path.as_str()) else {
            continue;
        };
        let mut stats = MaybeUninit::uninit();
        // statvfs 成功时会初始化整个结构。路径和输出缓冲区在调用期间有效。
        if unsafe { libc::statvfs(cpath.as_ptr(), stats.as_mut_ptr()) } != 0 {
            continue;
        }
        let stats = unsafe { stats.assume_init() };
        if stats.f_blocks == 0 {
            continue;
        }
        let total = (stats.f_blocks as u128 * stats.f_frsize as u128).min(u64::MAX as u128) as u64;
        let avail = (stats.f_bavail as u128 * stats.f_frsize as u128).min(total as u128) as u64;
        result.push(
            json!({"mountpoint":path,"fs_type":kind,"total_bytes":total,"available_bytes":avail}),
        );
        if result.len() == 64 {
            break;
        }
    }
    Ok(result)
}
