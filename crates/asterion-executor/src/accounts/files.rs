use std::ffi::{CString, OsStr};
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ExecError;

use super::Account;

const KEY_FILE: &str = "authorized_keys";
const SSH_DIR: &str = ".ssh";
const MAX_KEYS_BYTES: u64 = 128 * 1024;
const DIR_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
const READ_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
const WRITE_FLAGS: libc::c_int =
    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct AuthorizedKeys {
    ssh: File,
    pub(crate) bytes: Vec<u8>,
    version: Option<Version>,
}

#[derive(PartialEq, Eq)]
struct Version {
    device: u64,
    inode: u64,
    modified: i64,
    modified_ns: i64,
    changed: i64,
    changed_ns: i64,
    size: u64,
}

impl From<&Metadata> for Version {
    fn from(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            modified: metadata.mtime(),
            modified_ns: metadata.mtime_nsec(),
            changed: metadata.ctime(),
            changed_ns: metadata.ctime_nsec(),
            size: metadata.len(),
        }
    }
}

impl AuthorizedKeys {
    pub(crate) fn read(
        user: &Account,
        mutating: bool,
        create_ssh: bool,
    ) -> Result<Option<Self>, ExecError> {
        let home = match open_home(Path::new(&user.home)) {
            Ok(home) => home,
            Err(error) if !mutating && error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(file_error(error)),
        };
        let home_stat = home.metadata().map_err(file_error)?;
        if !trusted(&home_stat, user.uid, mutating) {
            return Err(ExecError::DENIED);
        }

        let ssh = match open_at(&home, OsStr::new(SSH_DIR), DIR_FLAGS, 0) {
            Ok(ssh) => ssh,
            Err(error) if error.kind() == io::ErrorKind::NotFound && create_ssh => {
                create_directory(&home, user)?
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !mutating => return Ok(None),
            Err(error) => return Err(file_error(error)),
        };
        if !trusted(&ssh.metadata().map_err(file_error)?, user.uid, mutating) {
            return Err(ExecError::DENIED);
        }

        let file = open_current(&ssh, user.uid, mutating)?;
        let mut bytes = Vec::new();
        let mut version = None;
        if let Some((file, original)) = file {
            (&file)
                .take(MAX_KEYS_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(file_error)?;
            if bytes.len() as u64 > MAX_KEYS_BYTES {
                return Err(ExecError::new("output_too_large", "公钥文件超出读取上限"));
            }
            let latest = Version::from(&file.metadata().map_err(file_error)?);
            if latest != original {
                return Err(conflict());
            }
            version = Some(original);
        }
        Ok(Some(Self {
            ssh,
            bytes,
            version,
        }))
    }

    pub(crate) fn replace(
        &self,
        user: &Account,
        replacing: &[u8],
        backup_dir: &Path,
    ) -> Result<(), ExecError> {
        if replacing.len() as u64 > MAX_KEYS_BYTES {
            return Err(ExecError::new("output_too_large", "公钥文件超出写入上限"));
        }
        write_backup(backup_dir, &user.username, "authorized-keys", &self.bytes)?;
        let name = CString::new(format!(".asterion-authorized-keys-{}", unique_suffix()))
            .map_err(|_| ExecError::FAILED)?;
        let raw = OsStr::from_bytes(name.as_bytes());
        let mut file = open_at(&self.ssh, raw, WRITE_FLAGS, 0o600).map_err(file_error)?;
        let mut temp = PendingFile {
            folder: &self.ssh,
            path: name,
            renamed: false,
        };
        file.write_all(replacing).map_err(file_error)?;
        // The temporary file is an exclusive regular file in the captured directory.
        if unsafe { libc::fchown(file.as_raw_fd(), user.uid, user.gid) } != 0 {
            return Err(file_error(io::Error::last_os_error()));
        }
        file.sync_all().map_err(file_error)?;

        let latest = open_current(&self.ssh, user.uid, true)?.map(|(_, version)| version);
        if latest != self.version {
            return Err(conflict());
        }
        rename_into_place(&self.ssh, &temp.path)?;
        temp.renamed = true;
        self.ssh.sync_all().map_err(file_error)?;
        Ok(())
    }
}

struct PendingFile<'a> {
    folder: &'a File,
    path: CString,
    renamed: bool,
}

impl Drop for PendingFile<'_> {
    fn drop(&mut self) {
        if !self.renamed {
            // Only the uniquely named file created by this transaction is removed.
            unsafe { libc::unlinkat(self.folder.as_raw_fd(), self.path.as_ptr(), 0) };
        }
    }
}

fn open_home(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut folder = File::open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => folder = open_at(&folder, name, DIR_FLAGS, 0)?,
            _ => return Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }
    Ok(folder)
}

fn open_at(
    folder: &File,
    name: &OsStr,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<File> {
    let name =
        CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // The path is relative to the already open directory, and symbolic links are refused.
    let fd = unsafe { libc::openat(folder.as_raw_fd(), name.as_ptr(), flags, mode) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn create_directory(home: &File, user: &Account) -> Result<File, ExecError> {
    let name = c".ssh";
    // Only the direct child .ssh of this verified home can be created.
    let created = unsafe { libc::mkdirat(home.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
    if !created && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
        return Err(file_error(io::Error::last_os_error()));
    }
    let dir = open_at(home, OsStr::new(SSH_DIR), DIR_FLAGS, 0).map_err(file_error)?;
    if created && unsafe { libc::fchown(dir.as_raw_fd(), user.uid, user.gid) } != 0 {
        return Err(file_error(io::Error::last_os_error()));
    }
    Ok(dir)
}

fn open_current(
    folder: &File,
    uid: u32,
    mutating: bool,
) -> Result<Option<(File, Version)>, ExecError> {
    let file = match open_at(folder, OsStr::new(KEY_FILE), READ_FLAGS, 0) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error(error)),
    };
    let metadata = file.metadata().map_err(file_error)?;
    if !metadata.is_file() || metadata.nlink() != 1 || !trusted(&metadata, uid, mutating) {
        return Err(ExecError::DENIED);
    }
    Ok(Some((file, Version::from(&metadata))))
}

fn trusted(metadata: &Metadata, uid: u32, mutating: bool) -> bool {
    metadata.mode() & 0o022 == 0 && (metadata.uid() == uid || !mutating && metadata.uid() == 0)
}

fn rename_into_place(folder: &File, temporary: &CString) -> Result<(), ExecError> {
    // Both entry names are within the captured .ssh directory. renameat does not follow
    // the existing target entry and preserves the atomic visibility of authorized_keys.
    if unsafe {
        libc::renameat(
            folder.as_raw_fd(),
            temporary.as_ptr(),
            folder.as_raw_fd(),
            c"authorized_keys".as_ptr(),
        )
    } != 0
    {
        return Err(file_error(io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) fn write_backup(
    dir: &Path,
    target: &str,
    action: &str,
    bytes: &[u8],
) -> Result<(), ExecError> {
    let folder = open_private_backup(dir)?;
    let name = format!("{target}.{action}.{}.backup", unique_suffix());
    let mut file = open_at(&folder, OsStr::new(&name), WRITE_FLAGS, 0o600).map_err(file_error)?;
    file.write_all(bytes).map_err(file_error)?;
    file.sync_all().map_err(file_error)?;
    folder.sync_all().map_err(file_error)?;
    Ok(())
}

pub(crate) fn validate_backup_dir(dir: &Path) -> Result<(), ExecError> {
    open_private_backup(dir).map(|_| ())
}

fn open_private_backup(dir: &Path) -> Result<File, ExecError> {
    if !dir.is_absolute() {
        return Err(ExecError::POLICY);
    }
    let folder = open_home(dir).map_err(file_error)?;
    let metadata = folder.metadata().map_err(file_error)?;
    // Use the same verified directory handle for opening the backup file.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(ExecError::POLICY);
    }
    Ok(folder)
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{sequence}", std::process::id())
}

fn conflict() -> ExecError {
    ExecError::new("conflict", "公钥文件已变化，请刷新后重试")
}

fn file_error(error: io::Error) -> ExecError {
    match error.kind() {
        io::ErrorKind::NotFound => ExecError::new("not_found", "用户目录或公钥文件不存在"),
        io::ErrorKind::PermissionDenied => {
            ExecError::new("permission_denied", "无法读取或维护此公钥文件")
        }
        _ if error.raw_os_error() == Some(libc::ELOOP) => ExecError::DENIED,
        _ => ExecError::io(error),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    struct Fixture(std::path::PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn key_replacement_backs_up_original_and_refuses_symlink_targets() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let scratch = workspace.join(".local");
        std::fs::create_dir_all(&scratch).unwrap();
        let fixture = Fixture(scratch.join(format!("account-fixture-{}", unique_suffix())));
        let home = fixture.0.join("home");
        let ssh = home.join(".ssh");
        let backup = fixture.0.join("backup");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::create_dir(&backup).unwrap();
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = ssh.join("authorized_keys");
        std::fs::write(&path, b"# before\n").unwrap();
        let account = Account {
            username: "fixture-user".into(),
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            shell: String::new(),
            home: home.to_string_lossy().into_owned(),
            groups: Vec::new(),
            system: false,
        };
        let before = AuthorizedKeys::read(&account, true, false)
            .unwrap()
            .unwrap();
        before.replace(&account, b"# after\n", &backup).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"# after\n");
        let saved = std::fs::read_dir(backup).unwrap().next().unwrap().unwrap();
        assert_eq!(std::fs::read(saved.path()).unwrap(), b"# before\n");
        assert_eq!(saved.metadata().unwrap().mode() & 0o777, 0o600);

        let unrelated = fixture.0.join("unrelated");
        std::fs::write(&unrelated, b"untouched").unwrap();
        std::fs::remove_file(&path).unwrap();
        symlink(&unrelated, &path).unwrap();
        assert!(AuthorizedKeys::read(&account, true, false).is_err());
        assert_eq!(std::fs::read(unrelated).unwrap(), b"untouched");
    }
}
