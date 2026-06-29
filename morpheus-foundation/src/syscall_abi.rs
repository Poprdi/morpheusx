//! Syscall ABI constants — sole source of truth for `SYS_*` numbers.
//! Numbers are ABI-stable: they ship in compiled user binaries.
//!
//! # ABI is LOCKED and APPEND-ONLY
//! These numbers are encoded in every shipped user/std binary, so they can NEVER
//! be renumbered, reordered, or reused. To add a syscall:
//!
//! 1. add its `SYS_*` const below with the next free number (== `SYSCALL_COUNT`),
//! 2. append its name to the END of `SYSCALL_TABLE`,
//! 3. bump `SYSCALL_COUNT` by one.
//!
//! The `const _` check at the bottom fails to COMPILE if any existing number
//! changes, the order shifts, a number is duplicated/skipped, or the table and
//! count disagree — so the only edit the compiler accepts is a correct append.

// Core (0-9)
pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_YIELD: u64 = 3;
pub const SYS_ALLOC: u64 = 4;
pub const SYS_FREE: u64 = 5;
pub const SYS_GETPID: u64 = 6;
pub const SYS_KILL: u64 = 7;
/// `wait(idtype, id, *mut WaitStatus, options(WNOHANG)) -> reaped pid | 0 | -errno`.
/// Status lands in `WaitStatus`, pid on the value channel — kills the old
/// exit-code/signal/errno collision where one return tried to carry all three.
pub const SYS_WAIT: u64 = 8;
pub const SYS_SLEEP: u64 = 9;

// HelixFS (10-21)
pub const SYS_OPEN: u64 = 10;
pub const SYS_CLOSE: u64 = 11;
pub const SYS_SEEK: u64 = 12;
pub const SYS_STAT: u64 = 13;
pub const SYS_READDIR: u64 = 14;
pub const SYS_MKDIR: u64 = 15;
pub const SYS_UNLINK: u64 = 16;
pub const SYS_RENAME: u64 = 17;
pub const SYS_TRUNCATE: u64 = 18;
pub const SYS_SYNC: u64 = 19;
pub const SYS_SNAPSHOT: u64 = 20;
pub const SYS_VERSIONS: u64 = 21;

// System / process / memory (22-31)
pub const SYS_CLOCK: u64 = 22;
pub const SYS_SYSINFO: u64 = 23;
pub const SYS_GETPPID: u64 = 24;
/// `posix_spawn(*const SpawnArgs) -> pid | -errno`. Versioned `SpawnArgs` carries
/// path/argv/envp/cwd/file_actions[] so creation needs no follow-up syscalls.
pub const SYS_SPAWN: u64 = 25;
pub const SYS_MMAP: u64 = 26;
pub const SYS_MUNMAP: u64 = 27;
pub const SYS_DUP: u64 = 28;
pub const SYS_SYSLOG: u64 = 29;
pub const SYS_GETCWD: u64 = 30;
pub const SYS_CHDIR: u64 = 31;

// Networking (32-41) — raw NIC primitives.
pub const SYS_NIC_INFO: u64 = 32;
pub const SYS_NIC_TX: u64 = 33;
pub const SYS_NIC_RX: u64 = 34;
pub const SYS_NIC_LINK: u64 = 35;
pub const SYS_NIC_MAC: u64 = 36;
pub const SYS_NIC_REFILL: u64 = 37;
pub const SYS_NET: u64 = 38;
pub const SYS_DNS: u64 = 39;
pub const SYS_NET_CFG: u64 = 40;
pub const SYS_NET_POLL: u64 = 41;

// Device / mount (42-45) — stubs.
pub const SYS_IOCTL: u64 = 42;
pub const SYS_MOUNT: u64 = 43;
pub const SYS_UMOUNT: u64 = 44;
pub const SYS_POLL: u64 = 45;

// Persistence / introspection (46-51)
pub const SYS_PERSIST_PUT: u64 = 46;
pub const SYS_PERSIST_GET: u64 = 47;
pub const SYS_PERSIST_DEL: u64 = 48;
pub const SYS_PERSIST_LIST: u64 = 49;
pub const SYS_PERSIST_INFO: u64 = 50;
pub const SYS_PE_INFO: u64 = 51;

// Raw hardware primitives (52-62)
pub const SYS_PORT_IN: u64 = 52;
pub const SYS_PORT_OUT: u64 = 53;
pub const SYS_PCI_CFG_READ: u64 = 54;
pub const SYS_PCI_CFG_WRITE: u64 = 55;
pub const SYS_DMA_ALLOC: u64 = 56;
pub const SYS_DMA_FREE: u64 = 57;
pub const SYS_MAP_PHYS: u64 = 58;
pub const SYS_VIRT_TO_PHYS: u64 = 59;
pub const SYS_IRQ_ATTACH: u64 = 60;
pub const SYS_IRQ_ACK: u64 = 61;
pub const SYS_CACHE_FLUSH: u64 = 62;

// Display (63-64)
pub const SYS_FB_INFO: u64 = 63;
pub const SYS_FB_MAP: u64 = 64;

// Process management (65-68)
pub const SYS_PS: u64 = 65;
pub const SYS_SIGACTION: u64 = 66;
pub const SYS_SETPRIORITY: u64 = 67;
pub const SYS_GETPRIORITY: u64 = 68;

// CPU features / diagnostics (69-72)
pub const SYS_CPUID: u64 = 69;
pub const SYS_RDTSC: u64 = 70;
pub const SYS_BOOT_LOG: u64 = 71;
pub const SYS_MEMMAP: u64 = 72;

// Memory sharing / protection (73-74)
pub const SYS_SHM_GRANT: u64 = 73;
pub const SYS_MPROTECT: u64 = 74;

// Shell / IPC (75-78)
pub const SYS_PIPE: u64 = 75;
pub const SYS_DUP2: u64 = 76;
pub const SYS_SET_FG: u64 = 77;
pub const SYS_GETARGS: u64 = 78;

// Sync (79)
pub const SYS_FUTEX: u64 = 79;

// Threads (80-82)
/// `thread_create(entry, stack_top, arg, tls_base, ctid_ptr, flags(THREAD_DETACHED))
/// -> tid | -errno`. Shares the parent address space; TLS set at creation and
/// `ctid_ptr` registered as a CLONE_CHILD_CLEARTID-style clear-tid futex.
pub const SYS_THREAD_CREATE: u64 = 80;
pub const SYS_THREAD_EXIT: u64 = 81;
pub const SYS_THREAD_JOIN: u64 = 82;

pub const SYS_SIGRETURN: u64 = 83;

pub const SYS_MOUSE_READ: u64 = 84;

// Framebuffer control (85-90)
pub const SYS_FB_LOCK: u64 = 85;
pub const SYS_FB_UNLOCK: u64 = 86;
pub const SYS_FB_IS_LOCKED: u64 = 87;
pub const SYS_FB_PRESENT: u64 = 88;
pub const SYS_FB_BLIT: u64 = 89;
pub const SYS_FB_MARK_DIRTY: u64 = 90;

// Compositor (91-98)
pub const SYS_COMPOSITOR_SET: u64 = 91;
pub const SYS_WIN_SURFACE_LIST: u64 = 92;
pub const SYS_WIN_SURFACE_MAP: u64 = 93;
pub const SYS_MOUSE_FORWARD: u64 = 94;
pub const SYS_WIN_SURFACE_DIRTY_CLEAR: u64 = 95;
pub const SYS_TRY_WAIT: u64 = 96;
pub const SYS_FORWARD_INPUT: u64 = 97;
pub const SYS_SYSTEM_CONTROL: u64 = 98;

/// Non-blocking drain of the kernel keyboard event ring (raw PS/2 Set 1 bytes).
/// The compositor reads input through this instead of the stdin byte stream.
pub const SYS_KEYBOARD_READ: u64 = 99;

/// `arch_prctl(ARCH_SET_FS)` analogue: set the calling thread's TLS base (x86 FS
/// base). Userland owns the TCB / variant-II layout; the kernel only stores and
/// per-switch restores the opaque pointer. Returns 0, or EINVAL if non-canonical.
pub const SYS_SET_THREAD_POINTER: u64 = 100;

/// `getrandom(buf, len, flags) -> bytes_written`. Linux-shaped so a Rust std PAL
/// and mlibc map 1:1. `flags` bit0 = GRND_NONBLOCK (advisory; RDRAND rarely
/// blocks). ENOSYS if the platform has no hardware RNG.
pub const SYS_GETRANDOM: u64 = 101;

/// getrandom flag: do not block on entropy. Advisory on RDRAND-backed systems.
pub const GRND_NONBLOCK: u64 = 0x0001;

/// `volumes(buf_ptr, max) -> count`. Fills `VolumeInfo[min(count,max)]`; `max==0`
/// is a count-probe (mirrors `SYS_VERSIONS`).
pub const SYS_VOLUMES: u64 = 102;

/// `mounts(buf_ptr, max) -> count`. Fills `MountInfo[min(count,max)]`; same
/// probe convention as `SYS_VOLUMES`.
pub const SYS_MOUNTS: u64 = 103;

// std PAL freeze additions (104-128). Append-only. SYS_SPAWN(25)/SYS_WAIT(8)/
// SYS_THREAD_CREATE(80) were reshaped IN PLACE (no parallel _2 numbers), so std
// targets the same numbers forever; the entries below are the genuinely new ones.

/// `clock_gettime(clock_id, *mut Timespec) -> 0 | -errno`.
pub const SYS_CLOCK_GETTIME: u64 = 104;
/// `nanosleep(*const Timespec req, *mut Timespec rem) -> 0 | -errno`.
pub const SYS_NANOSLEEP: u64 = 105;
/// `fstat(fd, *mut FileStat) -> 0 | -errno`. Path-less stat by open fd.
pub const SYS_FSTAT: u64 = 106;
/// `thread_detach(tid) -> 0 | -errno`. Marks a sibling auto-reaping.
pub const SYS_THREAD_DETACH: u64 = 107;
/// `gettid() -> tid`. Per-thread id from the one pid/tid allocator.
pub const SYS_GETTID: u64 = 108;
/// `socket(domain, type, protocol) -> fd | -errno`. Socket is a unified `O_SOCKET` fd.
pub const SYS_SOCKET: u64 = 109;
/// `bind(fd, *const SockAddrStorage, addrlen) -> 0 | -errno`.
pub const SYS_BIND: u64 = 110;
/// `listen(fd, backlog) -> 0 | -errno`.
pub const SYS_LISTEN: u64 = 111;
/// `accept(fd, *mut SockAddrStorage, *mut u32 addrlen, flags) -> newfd | -errno`.
pub const SYS_ACCEPT: u64 = 112;
/// `connect(fd, *const SockAddrStorage, addrlen) -> 0 | -errno`.
pub const SYS_CONNECT: u64 = 113;
/// `sendto(fd, buf, len, flags, *const SockAddrStorage, addrlen) -> n | -errno`.
pub const SYS_SENDTO: u64 = 114;
/// `recvfrom(fd, buf, len, flags, *mut SockAddrStorage, *mut u32 addrlen) -> n | -errno`.
pub const SYS_RECVFROM: u64 = 115;
/// `getsockname(fd, *mut SockAddrStorage, *mut u32 addrlen) -> 0 | -errno`.
pub const SYS_GETSOCKNAME: u64 = 116;
/// `getpeername(fd, *mut SockAddrStorage, *mut u32 addrlen) -> 0 | -errno`.
pub const SYS_GETPEERNAME: u64 = 117;
/// `setsockopt(fd, level, optname, *const optval, optlen) -> 0 | -errno`.
pub const SYS_SETSOCKOPT: u64 = 118;
/// `getsockopt(fd, level, optname, *mut optval, *mut u32 optlen) -> 0 | -errno`.
pub const SYS_GETSOCKOPT: u64 = 119;
/// `shutdown(fd, how) -> 0 | -errno`.
pub const SYS_SHUTDOWN: u64 = 120;
/// `epoll_create(flags) -> epfd | -errno`. The readiness instance is itself a fd.
pub const SYS_EPOLL_CREATE: u64 = 121;
/// `epoll_ctl(epfd, op, fd, *const EpollEvent) -> 0 | -errno`.
pub const SYS_EPOLL_CTL: u64 = 122;
/// `epoll_wait(epfd, *mut EpollEvent, maxevents, timeout_ms) -> nready | -errno`.
pub const SYS_EPOLL_WAIT: u64 = 123;
/// `getenv(buf_ptr, buf_len) -> total_block_bytes | -errno`. NUL-separated
/// `KEY=VALUE\0` records (same shape as `SYS_GETARGS`); `buf_len==0` probes.
pub const SYS_GETENV: u64 = 124;
/// `fsync(fd, flags) -> 0 | -errno`. `FSYNC_DATAONLY` skips metadata (fdatasync).
pub const SYS_FSYNC: u64 = 125;
/// `ftruncate(fd, new_len) -> 0 | -errno`. fd-based set-len (`SYS_TRUNCATE(18)` is path-based).
pub const SYS_FTRUNCATE: u64 = 126;
/// `fcntl(fd, cmd, arg) -> ret | -errno`. Runtime FD_CLOEXEC/O_NONBLOCK + CLOEXEC-dup.
pub const SYS_FCNTL: u64 = 127;
/// `rmdir(path_ptr, path_len) -> 0 | -errno`. Dirs only (`SYS_UNLINK(16)` is files-only).
pub const SYS_RMDIR: u64 = 128;

// Seek whence constants.
pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

// Append-only manifest of every syscall number, in ABI order. The const check
// below enforces that the Nth entry has value N, making any renumber, reorder,
// insertion, gap, duplicate, or table/count mismatch a compile error.

/// Number of defined syscalls. Bump by exactly one when appending.
pub const SYSCALL_COUNT: usize = 129;

/// Every `SYS_*` number in ABI order. Length is pinned to `SYSCALL_COUNT`, so a
/// missing/extra entry is itself a compile error.
pub const SYSCALL_TABLE: [u64; SYSCALL_COUNT] = [
    SYS_EXIT,
    SYS_WRITE,
    SYS_READ,
    SYS_YIELD,
    SYS_ALLOC,
    SYS_FREE,
    SYS_GETPID,
    SYS_KILL,
    SYS_WAIT,
    SYS_SLEEP,
    SYS_OPEN,
    SYS_CLOSE,
    SYS_SEEK,
    SYS_STAT,
    SYS_READDIR,
    SYS_MKDIR,
    SYS_UNLINK,
    SYS_RENAME,
    SYS_TRUNCATE,
    SYS_SYNC,
    SYS_SNAPSHOT,
    SYS_VERSIONS,
    SYS_CLOCK,
    SYS_SYSINFO,
    SYS_GETPPID,
    SYS_SPAWN,
    SYS_MMAP,
    SYS_MUNMAP,
    SYS_DUP,
    SYS_SYSLOG,
    SYS_GETCWD,
    SYS_CHDIR,
    SYS_NIC_INFO,
    SYS_NIC_TX,
    SYS_NIC_RX,
    SYS_NIC_LINK,
    SYS_NIC_MAC,
    SYS_NIC_REFILL,
    SYS_NET,
    SYS_DNS,
    SYS_NET_CFG,
    SYS_NET_POLL,
    SYS_IOCTL,
    SYS_MOUNT,
    SYS_UMOUNT,
    SYS_POLL,
    SYS_PERSIST_PUT,
    SYS_PERSIST_GET,
    SYS_PERSIST_DEL,
    SYS_PERSIST_LIST,
    SYS_PERSIST_INFO,
    SYS_PE_INFO,
    SYS_PORT_IN,
    SYS_PORT_OUT,
    SYS_PCI_CFG_READ,
    SYS_PCI_CFG_WRITE,
    SYS_DMA_ALLOC,
    SYS_DMA_FREE,
    SYS_MAP_PHYS,
    SYS_VIRT_TO_PHYS,
    SYS_IRQ_ATTACH,
    SYS_IRQ_ACK,
    SYS_CACHE_FLUSH,
    SYS_FB_INFO,
    SYS_FB_MAP,
    SYS_PS,
    SYS_SIGACTION,
    SYS_SETPRIORITY,
    SYS_GETPRIORITY,
    SYS_CPUID,
    SYS_RDTSC,
    SYS_BOOT_LOG,
    SYS_MEMMAP,
    SYS_SHM_GRANT,
    SYS_MPROTECT,
    SYS_PIPE,
    SYS_DUP2,
    SYS_SET_FG,
    SYS_GETARGS,
    SYS_FUTEX,
    SYS_THREAD_CREATE,
    SYS_THREAD_EXIT,
    SYS_THREAD_JOIN,
    SYS_SIGRETURN,
    SYS_MOUSE_READ,
    SYS_FB_LOCK,
    SYS_FB_UNLOCK,
    SYS_FB_IS_LOCKED,
    SYS_FB_PRESENT,
    SYS_FB_BLIT,
    SYS_FB_MARK_DIRTY,
    SYS_COMPOSITOR_SET,
    SYS_WIN_SURFACE_LIST,
    SYS_WIN_SURFACE_MAP,
    SYS_MOUSE_FORWARD,
    SYS_WIN_SURFACE_DIRTY_CLEAR,
    SYS_TRY_WAIT,
    SYS_FORWARD_INPUT,
    SYS_SYSTEM_CONTROL,
    SYS_KEYBOARD_READ,
    SYS_SET_THREAD_POINTER,
    SYS_GETRANDOM,
    SYS_VOLUMES,
    SYS_MOUNTS,
    SYS_CLOCK_GETTIME,
    SYS_NANOSLEEP,
    SYS_FSTAT,
    SYS_THREAD_DETACH,
    SYS_GETTID,
    SYS_SOCKET,
    SYS_BIND,
    SYS_LISTEN,
    SYS_ACCEPT,
    SYS_CONNECT,
    SYS_SENDTO,
    SYS_RECVFROM,
    SYS_GETSOCKNAME,
    SYS_GETPEERNAME,
    SYS_SETSOCKOPT,
    SYS_GETSOCKOPT,
    SYS_SHUTDOWN,
    SYS_EPOLL_CREATE,
    SYS_EPOLL_CTL,
    SYS_EPOLL_WAIT,
    SYS_GETENV,
    SYS_FSYNC,
    SYS_FTRUNCATE,
    SYS_FCNTL,
    SYS_RMDIR,
];

const _: () = {
    let mut i = 0;
    while i < SYSCALL_TABLE.len() {
        assert!(
            SYSCALL_TABLE[i] == i as u64,
            "syscall ABI violation: numbers must be contiguous, ordered, and append-only"
        );
        i += 1;
    }
};
