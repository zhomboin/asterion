use std::collections::HashSet;
use std::ffi::CString;
use std::mem::MaybeUninit;

use serde::Serialize;

const MAX_MOUNTS: usize = 128;

#[derive(Serialize)]
pub(crate) struct FileSystem {
    mountpoint: String,
    fs_type: String,
    total_bytes: u64,
    available_bytes: u64,
}

pub(crate) fn from_mountinfo(contents: &str) -> Vec<FileSystem> {
    let mut seen = HashSet::new();
    let mut mounts = Vec::new();
    for line in contents.lines().rev() {
        let Some((path, fs_type)) = parse_mount(line) else {
            continue;
        };
        if !seen.insert(path.clone()) || !collectible(&path, fs_type) {
            continue;
        }
        if let Some((total_bytes, available_bytes)) = filesystem_usage(&path) {
            mounts.push(FileSystem {
                mountpoint: String::from_utf8_lossy(&path).into_owned(),
                fs_type: fs_type.to_owned(),
                total_bytes,
                available_bytes,
            });
        }
        if mounts.len() >= MAX_MOUNTS {
            break;
        }
    }
    mounts.sort_by(|first, second| first.mountpoint.cmp(&second.mountpoint));
    mounts
}

fn parse_mount(line: &str) -> Option<(Vec<u8>, &str)> {
    let (attributes, filesystem) = line.split_once(" - ")?;
    let mountpoint = attributes.split_whitespace().nth(4)?;
    if !mountpoint.starts_with('/') || mountpoint.len() > 1024 {
        return None;
    }
    let fs_type = filesystem.split_whitespace().next()?;
    Some((decode_mountpoint(mountpoint.as_bytes()), fs_type))
}

fn decode_mountpoint(path: &[u8]) -> Vec<u8> {
    let mut path = path;
    let mut output = Vec::with_capacity(path.len());
    while !path.is_empty() {
        let (value, length) = match path {
            [b'\\', b'0', b'4', b'0', ..] => (b' ', 4),
            [b'\\', b'0', b'1', b'1', ..] => (b'\t', 4),
            [b'\\', b'0', b'1', b'2', ..] => (b'\n', 4),
            [b'\\', b'1', b'3', b'4', ..] => (b'\\', 4),
            [value, ..] => (*value, 1),
            [] => unreachable!(),
        };
        output.push(value);
        path = &path[length..];
    }
    output
}

fn collectible(path: &[u8], fs_type: &str) -> bool {
    path == b"/"
        || matches!(
            fs_type,
            "ext2"
                | "ext3"
                | "ext4"
                | "xfs"
                | "btrfs"
                | "f2fs"
                | "zfs"
                | "vfat"
                | "exfat"
                | "ntfs"
                | "ntfs3"
                | "ufs"
                | "jfs"
                | "reiserfs"
                | "bcachefs"
        )
}

fn filesystem_usage(path: &[u8]) -> Option<(u64, u64)> {
    let pathname = CString::new(path).ok()?;
    let mut result = MaybeUninit::<libc::statvfs>::uninit();
    // The file name is NUL terminated and statvfs initializes the output on success.
    if unsafe { libc::statvfs(pathname.as_ptr(), result.as_mut_ptr()) } != 0 {
        return None;
    }
    let result = unsafe { result.assume_init() };
    let block = u64::try_from(u128::from(result.f_frsize)).ok()?;
    let total = u128::from(result.f_blocks) * u128::from(block);
    let available = u128::from(result.f_bavail) * u128::from(block);
    Some((
        u64::try_from(total).ok()?,
        u64::try_from(available.min(total)).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_handles_kernel_escaping_and_filters_pseudo_filesystems() {
        let (path, fs_type) =
            parse_mount(r"19 1 8:2 / /mnt/with\040space\134path rw - ext4 /dev/sda2 rw").unwrap();
        assert_eq!(path, b"/mnt/with space\\path");
        assert_eq!(fs_type, "ext4");
        assert!(collectible(&path, fs_type));
        assert!(!collectible(b"/proc", "proc"));
        assert!(!collectible(b"/mnt/c", "9p"));
        assert!(collectible(b"/", "overlay"));
    }
}
