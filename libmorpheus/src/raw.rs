//! Raw syscall wrappers — thin inline asm around the `syscall` instruction.

// Canonical SYS_* numbers live in morpheus-foundation::syscall_abi. Re-exported
// here for source compatibility with existing `libmorpheus::raw::SYS_*` callers
// and to guarantee the kernel-side dispatcher (hwinit) and this userland side
// cannot drift.
pub use morpheus_foundation::syscall_abi::*;

// futex ops + sysctl modes are canonical in morpheus-foundation — single source.
pub use morpheus_foundation::flags::{
    FUTEX_WAIT, FUTEX_WAKE, SYSCTL_REBOOT_FORCE, SYSCTL_REBOOT_GRACEFUL, SYSCTL_SHUTDOWN_FORCE,
    SYSCTL_SHUTDOWN_GRACEFUL, SYSCTL_SHUTDOWN_PANIC,
};

/// # Safety
/// `nr` must be a valid syscall number for the kernel ABI. The caller is
/// responsible for upholding any invariants the selected syscall requires.
#[inline(always)]
pub unsafe fn syscall0(nr: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall1(nr: u64, a1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("r10") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        out("rcx") _,
        out("r11") _,
        out("r8") _,
        out("r9") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        out("rcx") _,
        out("r11") _,
        out("r9") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

/// # Safety
/// Caller upholds the ABI contract for `nr`; any pointer args must be valid for the syscall's access pattern.
#[inline(always)]
pub unsafe fn syscall6(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        out("rcx") _,
        out("r11") _,
        out("xmm0") _, out("xmm1") _, out("xmm2") _,
        out("xmm3") _, out("xmm4") _, out("xmm5") _,
        options(nostack),
    );
    ret
}

// Thin per-number wrappers for the std-PAL freeze additions (SYS 104..=128).
// Args stay `u64` so this layer pulls in no struct types; the typed pointer-
// handing wrappers live one layer up. SYS_SPAWN/SYS_WAIT/SYS_THREAD_CREATE were
// reshaped in place and keep their existing wrappers.
//
// # Safety (shared by all wrappers below)
// Same contract as `syscallN`: every pointer arg must be valid for the access its
// syscall documents, and any referenced struct must match the frozen ABI layout.

/// `SYS_CLOCK_GETTIME(clock_id, *mut Timespec) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_clock_gettime(clock_id: u64, ts: u64) -> u64 {
    syscall2(SYS_CLOCK_GETTIME, clock_id, ts)
}

/// `SYS_NANOSLEEP(*const Timespec req, *mut Timespec rem) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_nanosleep(req: u64, rem: u64) -> u64 {
    syscall2(SYS_NANOSLEEP, req, rem)
}

/// `SYS_FSTAT(fd, *mut FileStat) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_fstat(fd: u64, stat: u64) -> u64 {
    syscall2(SYS_FSTAT, fd, stat)
}

/// `SYS_THREAD_DETACH(tid) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_thread_detach(tid: u64) -> u64 {
    syscall1(SYS_THREAD_DETACH, tid)
}

/// `SYS_GETTID() -> tid`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_gettid() -> u64 {
    syscall0(SYS_GETTID)
}

/// `SYS_SOCKET(domain, type, protocol) -> fd | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_socket(domain: u64, ty: u64, protocol: u64) -> u64 {
    syscall3(SYS_SOCKET, domain, ty, protocol)
}

/// `SYS_BIND(fd, *const SockAddrStorage, addrlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_bind(fd: u64, addr: u64, addrlen: u64) -> u64 {
    syscall3(SYS_BIND, fd, addr, addrlen)
}

/// `SYS_LISTEN(fd, backlog) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_listen(fd: u64, backlog: u64) -> u64 {
    syscall2(SYS_LISTEN, fd, backlog)
}

/// `SYS_ACCEPT(fd, *mut SockAddrStorage, *mut u32 addrlen, flags) -> newfd | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_accept(fd: u64, addr: u64, addrlen: u64, flags: u64) -> u64 {
    syscall4(SYS_ACCEPT, fd, addr, addrlen, flags)
}

/// `SYS_CONNECT(fd, *const SockAddrStorage, addrlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_connect(fd: u64, addr: u64, addrlen: u64) -> u64 {
    syscall3(SYS_CONNECT, fd, addr, addrlen)
}

/// `SYS_SENDTO(fd, buf, len, flags, *const SockAddrStorage, addrlen) -> n | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_sendto(fd: u64, buf: u64, len: u64, flags: u64, addr: u64, addrlen: u64) -> u64 {
    syscall6(SYS_SENDTO, fd, buf, len, flags, addr, addrlen)
}

/// `SYS_RECVFROM(fd, buf, len, flags, *mut SockAddrStorage, *mut u32 addrlen) -> n | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_recvfrom(
    fd: u64,
    buf: u64,
    len: u64,
    flags: u64,
    addr: u64,
    addrlen: u64,
) -> u64 {
    syscall6(SYS_RECVFROM, fd, buf, len, flags, addr, addrlen)
}

/// `SYS_GETSOCKNAME(fd, *mut SockAddrStorage, *mut u32 addrlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_getsockname(fd: u64, addr: u64, addrlen: u64) -> u64 {
    syscall3(SYS_GETSOCKNAME, fd, addr, addrlen)
}

/// `SYS_GETPEERNAME(fd, *mut SockAddrStorage, *mut u32 addrlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_getpeername(fd: u64, addr: u64, addrlen: u64) -> u64 {
    syscall3(SYS_GETPEERNAME, fd, addr, addrlen)
}

/// `SYS_SETSOCKOPT(fd, level, optname, *const optval, optlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_setsockopt(fd: u64, level: u64, optname: u64, optval: u64, optlen: u64) -> u64 {
    syscall5(SYS_SETSOCKOPT, fd, level, optname, optval, optlen)
}

/// `SYS_GETSOCKOPT(fd, level, optname, *mut optval, *mut u32 optlen) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_getsockopt(fd: u64, level: u64, optname: u64, optval: u64, optlen: u64) -> u64 {
    syscall5(SYS_GETSOCKOPT, fd, level, optname, optval, optlen)
}

/// `SYS_SHUTDOWN(fd, how) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_shutdown(fd: u64, how: u64) -> u64 {
    syscall2(SYS_SHUTDOWN, fd, how)
}

/// `SYS_EPOLL_CREATE(flags) -> epfd | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_epoll_create(flags: u64) -> u64 {
    syscall1(SYS_EPOLL_CREATE, flags)
}

/// `SYS_EPOLL_CTL(epfd, op, fd, *const EpollEvent) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_epoll_ctl(epfd: u64, op: u64, fd: u64, event: u64) -> u64 {
    syscall4(SYS_EPOLL_CTL, epfd, op, fd, event)
}

/// `SYS_EPOLL_WAIT(epfd, *mut EpollEvent, maxevents, timeout_ms) -> nready | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_epoll_wait(epfd: u64, events: u64, maxevents: u64, timeout_ms: u64) -> u64 {
    syscall4(SYS_EPOLL_WAIT, epfd, events, maxevents, timeout_ms)
}

/// `SYS_GETENV(buf_ptr, buf_len) -> total_block_bytes | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_getenv(buf: u64, buf_len: u64) -> u64 {
    syscall2(SYS_GETENV, buf, buf_len)
}

/// `SYS_FSYNC(fd, flags) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_fsync(fd: u64, flags: u64) -> u64 {
    syscall2(SYS_FSYNC, fd, flags)
}

/// `SYS_FTRUNCATE(fd, new_len) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_ftruncate(fd: u64, new_len: u64) -> u64 {
    syscall2(SYS_FTRUNCATE, fd, new_len)
}

/// `SYS_FCNTL(fd, cmd, arg) -> ret | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_fcntl(fd: u64, cmd: u64, arg: u64) -> u64 {
    syscall3(SYS_FCNTL, fd, cmd, arg)
}

/// `SYS_RMDIR(path_ptr, path_len) -> 0 | -errno`.
///
/// # Safety
/// Pointer args must be valid for the access the syscall documents; referenced
/// structs must match the frozen ABI layout.
#[inline(always)]
pub unsafe fn sys_rmdir(path: u64, path_len: u64) -> u64 {
    syscall2(SYS_RMDIR, path, path_len)
}
