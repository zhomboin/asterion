use std::io;
use std::mem::MaybeUninit;

#[cfg(all(
    target_os = "linux",
    target_pointer_width = "64",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
))]
pub fn confine_process_group() -> io::Result<()> {
    const LD_WORD_ABS: u32 = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
    const JUMP_EQUAL: u32 = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
    const RETURN: u32 = libc::BPF_RET | libc::BPF_K;
    const DENY: u32 = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
    const AUDIT_ARCH_AARCH64: u32 = 0xc000_00b7;
    let architecture = if cfg!(target_arch = "x86_64") {
        AUDIT_ARCH_X86_64
    } else {
        AUDIT_ARCH_AARCH64
    };
    let instruction = |code, k, jt, jf| libc::sock_filter {
        code: code as u16,
        jt,
        jf,
        k,
    };
    let mut filter = [
        instruction(
            LD_WORD_ABS,
            std::mem::offset_of!(libc::seccomp_data, arch) as u32,
            0,
            0,
        ),
        instruction(JUMP_EQUAL, architecture, 1, 0),
        instruction(RETURN, KILL, 0, 0),
        instruction(
            LD_WORD_ABS,
            std::mem::offset_of!(libc::seccomp_data, nr) as u32,
            0,
            0,
        ),
        // x86-64 与 x32 使用相同的审计架构标识，须单独排除其编号。
        #[cfg(target_arch = "x86_64")]
        instruction(libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K, 1 << 30, 0, 1),
        #[cfg(target_arch = "x86_64")]
        instruction(RETURN, KILL, 0, 0),
        #[cfg(target_arch = "x86_64")]
        instruction(libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K, 512, 0, 2),
        #[cfg(target_arch = "x86_64")]
        instruction(libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K, 548, 1, 0),
        #[cfg(target_arch = "x86_64")]
        instruction(RETURN, KILL, 0, 0),
        instruction(JUMP_EQUAL, libc::SYS_setsid as u32, 0, 1),
        instruction(RETURN, DENY, 0, 0),
        instruction(JUMP_EQUAL, libc::SYS_setpgid as u32, 0, 1),
        instruction(RETURN, DENY, 0, 0),
        instruction(RETURN, libc::SECCOMP_RET_ALLOW, 0, 0),
    ];
    let program = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    // pre_exec 中仅使用栈数据与系统调用。过滤器安装失败时拒绝启动 shell。
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1_u64, 0_u64, 0_u64, 0_u64) } != 0
        || unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                libc::SECCOMP_SET_MODE_FILTER,
                0_u32,
                &program,
            )
        } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(all(
    target_os = "linux",
    target_pointer_width = "64",
    target_endian = "little",
    any(target_arch = "x86_64", target_arch = "aarch64"),
)))]
pub fn confine_process_group() -> io::Result<()> {
    Err(io::Error::from_raw_os_error(libc::ENOTSUP))
}

pub fn leader_exited(leader: i32) -> io::Result<bool> {
    let mut event = MaybeUninit::<libc::siginfo_t>::zeroed();
    // WNOWAIT 检查退出状态，同时保留组长 PID 直到完成清理。
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            leader as libc::id_t,
            event.as_mut_ptr(),
            libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { event.assume_init().si_pid() } == leader)
}
