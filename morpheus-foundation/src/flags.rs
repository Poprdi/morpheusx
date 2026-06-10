//! Canonical scalar ABI flags / request codes shared kernel↔userland.
//!
//! Like the syscall numbers and boundary structs, these values are part of the
//! frozen seam: both sides must agree, so they live here exactly once and
//! consumers re-export rather than re-declare. (Net subcommand codes live in a
//! separate pass.)

/// `open(path, flags)` bits. The low five are user-facing; `O_DIR`/`O_AT_LSN`/
/// `O_PIPE_*` are kernel-internal fd markers that share the namespace.
pub mod open_flags {
    pub const O_READ: u32 = 0x01;
    pub const O_WRITE: u32 = 0x02;
    pub const O_CREATE: u32 = 0x04;
    pub const O_TRUNC: u32 = 0x10;
    pub const O_APPEND: u32 = 0x20;
    pub const O_DIR: u32 = 0x40;
    pub const O_AT_LSN: u32 = 0x80;
    pub const O_PIPE_READ: u32 = 0x100;
    pub const O_PIPE_WRITE: u32 = 0x200;
}

/// `mmap`/`mprotect`/`shm_grant` protection bitmap. Read is implicit on x86-64;
/// `PROT_WRITE` is bit 0, `PROT_EXEC` is bit 1.
pub const PROT_READ: u64 = 0;
pub const PROT_WRITE: u64 = 1;
pub const PROT_EXEC: u64 = 2;

/// `map_phys(phys, pages, flags)` bits: bit 0 = writable, bit 1 = uncacheable.
pub const MAP_PHYS_WRITE: u64 = 1;
pub const MAP_PHYS_UNCACHEABLE: u64 = 2;

/// `futex(addr, op, ...)` operations.
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;

/// `ioctl(fd, cmd, arg)` request codes (Linux-numbered for std/mlibc 1:1).
pub const IOCTL_FIONREAD: u64 = 0x541B;
pub const IOCTL_FIONBIO: u64 = 0x5421;
pub const IOCTL_TIOCGWINSZ: u64 = 0x5413;

/// `poll` event bits (in/out fields of a pollfd).
pub const POLLIN: i16 = 0x0001;
pub const POLLOUT: i16 = 0x0004;
pub const POLLERR: i16 = 0x0008;

/// Signal numbers (`kill`, `sigaction`). POSIX-numbered.
pub mod signal {
    pub const SIGINT: u8 = 2;
    pub const SIGKILL: u8 = 9;
    pub const SIGSEGV: u8 = 11;
    pub const SIGTERM: u8 = 15;
    pub const SIGCHLD: u8 = 17;
    pub const SIGCONT: u8 = 18;
    pub const SIGSTOP: u8 = 19;
}

/// `system_control(mode)` modes.
pub const SYSCTL_REBOOT_GRACEFUL: u64 = 0;
pub const SYSCTL_REBOOT_FORCE: u64 = 1;
pub const SYSCTL_SHUTDOWN_GRACEFUL: u64 = 2;
pub const SYSCTL_SHUTDOWN_FORCE: u64 = 3;
pub const SYSCTL_SHUTDOWN_PANIC: u64 = 4;
