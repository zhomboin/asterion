use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Instant;

use asterion_protocol::operation::ProcessSignal;

use crate::{ExecError, run::ResultData};

pub(crate) fn signal_process(
    pid: u32,
    start_ticks: u64,
    signal: ProcessSignal,
) -> Result<ResultData, ExecError> {
    let started = Instant::now();
    // pidfd holds a reference to the exact process even if its numeric PID
    // becomes available for another process before the signal is delivered.
    let parent = unsafe { libc::getppid() };
    let pid = i32::try_from(pid).map_err(|_| ExecError::INVALID)?;
    if pid <= 1 || pid as u32 == std::process::id() || pid == parent {
        return Err(ExecError::DENIED);
    }
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
    if fd < 0 {
        return Err(process_error(std::io::Error::last_os_error()));
    }
    // pidfd_open returned a fresh owned file descriptor.
    let target = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let latest = asterion_collector::process_start_ticks(pid as u32).map_err(|error| {
        if error.code == "permission_denied" {
            ExecError::DENIED
        } else {
            ExecError::STALE_TARGET
        }
    })?;
    if latest != start_ticks {
        return Err(ExecError::STALE_TARGET);
    }

    let signum = match signal {
        ProcessSignal::Term => libc::SIGTERM,
        ProcessSignal::Kill => libc::SIGKILL,
    };
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            target.as_raw_fd(),
            signum,
            std::ptr::null::<libc::siginfo_t>(),
            0u32,
        )
    };
    if result != 0 {
        return Err(process_error(std::io::Error::last_os_error()));
    }
    Ok(ResultData::empty(started))
}

fn process_error(error: std::io::Error) -> ExecError {
    match error.raw_os_error() {
        Some(libc::EPERM | libc::EACCES) => ExecError::DENIED,
        Some(libc::ESRCH) => ExecError::STALE_TARGET,
        Some(libc::ENOSYS | libc::EINVAL) => {
            ExecError::new("unavailable", "当前 Linux 内核不支持安全的进程操作")
        }
        _ => ExecError::FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_rejected_before_any_signal() {
        let result = signal_process(std::process::id(), 1, ProcessSignal::Term);
        assert_eq!(result.err().unwrap().code, "operation_denied");
    }
}
