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
    /// `create_new` = `O_CREATE | O_EXCL` ⇒ `EEXIST` if the path exists.
    pub const O_EXCL: u32 = 0x08;
    pub const O_TRUNC: u32 = 0x10;
    pub const O_APPEND: u32 = 0x20;
    pub const O_DIR: u32 = 0x40;
    pub const O_AT_LSN: u32 = 0x80;
    pub const O_PIPE_READ: u32 = 0x100;
    pub const O_PIPE_WRITE: u32 = 0x200;
    /// Kernel-internal marker: this fd is a socket (dispatch by flag, not number).
    pub const O_SOCKET: u32 = 0x400;
    pub const O_NONBLOCK: u32 = 0x800;
    pub const O_CLOEXEC: u32 = 0x80000;
}

/// `FileStat.mode` bits: `S_IFMT` selects the file type, low `0o7777` are perms.
/// `Permissions::readonly()` == `(mode & 0o222) == 0`.
pub mod mode {
    pub const S_IFMT: u32 = 0xF000;
    pub const S_IFSOCK: u32 = 0xC000;
    pub const S_IFLNK: u32 = 0xA000;
    pub const S_IFREG: u32 = 0x8000;
    pub const S_IFBLK: u32 = 0x6000;
    pub const S_IFDIR: u32 = 0x4000;
    pub const S_IFCHR: u32 = 0x2000;
    pub const S_IFIFO: u32 = 0x1000;
}

/// `DirEntry.d_type` values (Linux `DT_*`).
pub mod dirent_type {
    pub const DT_UNKNOWN: u8 = 0;
    pub const DT_FIFO: u8 = 1;
    pub const DT_CHR: u8 = 2;
    pub const DT_DIR: u8 = 4;
    pub const DT_BLK: u8 = 6;
    pub const DT_REG: u8 = 8;
    pub const DT_LNK: u8 = 10;
    pub const DT_SOCK: u8 = 12;
}

/// `clock_gettime` clock ids.
pub const CLOCK_REALTIME: u64 = 0;
pub const CLOCK_MONOTONIC: u64 = 1;

/// `waitid` idtype selectors.
pub const P_ALL: u64 = 0;
pub const P_PID: u64 = 1;
pub const P_PGID: u64 = 2;

/// `waitid` option bits.
pub const WNOHANG: u64 = 1;
pub const WUNTRACED: u64 = 2;
pub const WCONTINUED: u64 = 8;

/// `SpawnFileAction.op` opcodes, replayed in array order.
pub const SPAWN_FA_OPEN: u32 = 0;
pub const SPAWN_FA_DUP2: u32 = 1;
pub const SPAWN_FA_CLOSE: u32 = 2;
pub const SPAWN_FA_CHDIR: u32 = 3;
/// Install `/dev/null` (`FdKind::Null`: read→EOF, write→discard) at `fd`. Backs
/// `Stdio::null()` without a real `/dev` filesystem. Append-only new opcode.
pub const SPAWN_FA_NULL: u32 = 4;
/// `SpawnArgs.flags` bit0: start from an empty fd table instead of the inherited one.
pub const SPAWN_CLEAR_FDS: u32 = 1 << 0;

/// `thread_create`/`thread_detach` flags.
pub const THREAD_DETACHED: u64 = 1;

/// `fsync` flags: set ⇒ data-only (fdatasync), clear ⇒ full fsync (sync_all).
pub const FSYNC_DATAONLY: u64 = 1;

/// `fcntl` commands and the single `FD_CLOEXEC` fd-flag bit.
pub const F_DUPFD: u64 = 0;
pub const F_GETFD: u64 = 1;
pub const F_SETFD: u64 = 2;
pub const F_GETFL: u64 = 3;
pub const F_SETFL: u64 = 4;
pub const F_DUPFD_CLOEXEC: u64 = 1030;
pub const FD_CLOEXEC: u64 = 1;

/// `epoll_create`/`epoll_ctl` flags and ops.
pub const EPOLL_CLOEXEC: u64 = 0x80000;
pub const EPOLL_CTL_ADD: u64 = 1;
pub const EPOLL_CTL_DEL: u64 = 2;
pub const EPOLL_CTL_MOD: u64 = 3;

/// `epoll_event.events` bits.
pub const EPOLLIN: u32 = 0x001;
pub const EPOLLPRI: u32 = 0x002;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLRDHUP: u32 = 0x2000;
pub const EPOLLONESHOT: u32 = 0x4000_0000;
pub const EPOLLET: u32 = 0x8000_0000;

/// Reserved now for a future `lstat`/`statat` NOFOLLOW form.
pub const AT_SYMLINK_NOFOLLOW: u64 = 0x100;

/// `mmap`/`mprotect`/`shm_grant` protection bitmap. Read is implicit on x86-64;
/// `PROT_WRITE` is bit 0, `PROT_EXEC` is bit 1.
pub const PROT_READ: u64 = 0;
pub const PROT_WRITE: u64 = 1;
pub const PROT_EXEC: u64 = 2;
/// Guard sentinel — no access. Deviates from Linux's `PROT_NONE==0` because here
/// 0 already means read-only, so a dedicated bit is needed to express an
/// inaccessible mapping. Mapped supervisor-only: any ring-3 touch faults cleanly
/// (stack-overflow guard).
pub const PROT_NONE: u64 = 0x4;

/// `mmap(pages, prot, flags, addr)` flags. Linux-numeric where they overlap so
/// std's `MAP_*` map 1:1; only anonymous private mappings are backed (file-backed
/// mmap is unsupported and silently treated as anonymous).
pub const MAP_SHARED: u64 = 0x01;
pub const MAP_PRIVATE: u64 = 0x02;
/// Map exactly at the supplied `addr` instead of letting the kernel place it.
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;

/// `map_phys(phys, pages, flags)` bits: bit 0 = writable, bit 1 = uncacheable.
pub const MAP_PHYS_WRITE: u64 = 1;
pub const MAP_PHYS_UNCACHEABLE: u64 = 2;

/// `futex(addr, op, val, timeout, ...)` operations. arg4 of `FUTEX_WAIT` is a
/// `*const Timespec` RELATIVE timeout vs `CLOCK_MONOTONIC` (NULL=forever),
/// returning 0 woken / `-ETIMEDOUT` expiry / `-EAGAIN` value-mismatch / `-EINTR`.
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;
pub const FUTEX_WAIT_BITSET: u64 = 9;
pub const FUTEX_WAKE_BITSET: u64 = 10;

/// `ioctl(fd, cmd, arg)` request codes (Linux-numbered for std/mlibc 1:1).
pub const IOCTL_FIONREAD: u64 = 0x541B;
pub const IOCTL_FIONBIO: u64 = 0x5421;
pub const IOCTL_TIOCGWINSZ: u64 = 0x5413;

/// `poll` event bits (in/out fields of a `PollFd`).
pub const POLLIN: i16 = 0x0001;
pub const POLLPRI: i16 = 0x0002;
pub const POLLOUT: i16 = 0x0004;
pub const POLLERR: i16 = 0x0008;
pub const POLLHUP: i16 = 0x0010;
pub const POLLNVAL: i16 = 0x0020;
pub const POLLRDHUP: i16 = 0x2000;

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
