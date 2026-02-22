//! Syscall dispatch handler — individual syscall implementations.
//!
//! Each `sys_*` function receives arguments already decoded by the assembly
//! trampoline and dispatched by `syscall_dispatch()` in `mod.rs`.
//!
//! # Calling convention
//!
//! All handlers are called with MS x64 ABI by the Rust dispatcher.
//! They receive arguments as plain `u64` values and return a `u64` result
//! (negative value = errno equivalent; 0 = success; positive = data).

use crate::process::scheduler::{exit_process, spawn_kernel_thread, SCHEDULER};
use crate::process::signals::Signal;
use crate::serial::{puts, put_hex64, put_hex32};

// ═══════════════════════════════════════════════════════════════════════════
// SYS_EXIT — terminate the current process
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_EXIT(code: i32)` — terminate the calling process.
///
/// Never returns.
pub unsafe fn sys_exit(code: u64) -> u64 {
    exit_process(code as i32);
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_WRITE — write to serial console
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_WRITE(ptr: *const u8, len: usize)` — write bytes to serial.
///
/// For now all writes go to COM1 serial; future: fd-based dispatch.
pub unsafe fn sys_write(ptr: u64, len: u64, _a3: u64) -> u64 {
    if ptr == 0 || len == 0 || len > 4096 {
        return u64::MAX; // -EINVAL
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    if let Ok(s) = core::str::from_utf8(bytes) {
        puts(s);
        len // bytes written
    } else {
        u64::MAX - 1 // -EBADMSG
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_READ — not yet implemented
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn sys_read(_fd: u64, _ptr: u64, _len: u64) -> u64 {
    u64::MAX - 37 // -ENOSYS
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_YIELD — voluntary context switch
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_YIELD()` — yield the rest of the current time slice.
///
/// This triggers a software-initiated context switch via the timer ISR
/// mechanism.  In the current kernel-mode-only world this is a no-op
/// (we have no user→kernel boundary to yield across).
pub unsafe fn sys_yield() -> u64 {
    // Fire a self-IPI or simply pulse INT 0x20 to invoke the timer ISR.
    // For now: no-op (single ring-0 cooperative yield is not yet needed).
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_GETPID
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn sys_getpid() -> u64 {
    SCHEDULER.current_pid() as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_KILL — send a signal to a process
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_KILL(pid: u32, signal: u8)` — send signal to process.
pub unsafe fn sys_kill(pid: u64, signum: u64) -> u64 {
    let sig = match Signal::from_u8(signum as u8) {
        Some(s) => s,
        None => return u64::MAX - 22, // -EINVAL
    };
    match SCHEDULER.send_signal(pid as u32, sig) {
        Ok(_) => 0,
        Err(_) => u64::MAX - 3, // -ESRCH
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_WAIT — wait for a child process (stub)
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn sys_wait(_pid: u64) -> u64 {
    u64::MAX - 37 // -ENOSYS (not yet implemented)
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SLEEP — sleep for N scheduler ticks (stub)
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn sys_sleep(_ticks: u64) -> u64 {
    // TODO: set BlockReason::Sleep(deadline) on current process once the
    // scheduler can unblock on TSC deadline.
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// HelixFS Syscall Stubs
//
// These will be wired to the VFS layer once a block device and mount
// table are initialized at runtime.  Until then they return -ENOSYS.
// ═══════════════════════════════════════════════════════════════════════════

const ENOSYS: u64 = u64::MAX - 37;

/// `SYS_OPEN(path_ptr, path_len, flags) → fd`
pub unsafe fn sys_fs_open(_path_ptr: u64, _path_len: u64, _flags: u64) -> u64 {
    ENOSYS
}

/// `SYS_CLOSE(fd) → 0`
pub unsafe fn sys_fs_close(_fd: u64) -> u64 {
    ENOSYS
}

/// `SYS_SEEK(fd, offset, whence) → new_offset`
pub unsafe fn sys_fs_seek(_fd: u64, _offset: u64, _whence: u64) -> u64 {
    ENOSYS
}

/// `SYS_STAT(path_ptr, path_len, stat_buf_ptr) → 0`
pub unsafe fn sys_fs_stat(_path_ptr: u64, _path_len: u64, _stat_buf: u64) -> u64 {
    ENOSYS
}

/// `SYS_READDIR(fd, entry_buf_ptr, max_entries) → count`
pub unsafe fn sys_fs_readdir(_fd: u64, _buf: u64, _max: u64) -> u64 {
    ENOSYS
}

/// `SYS_MKDIR(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_mkdir(_path_ptr: u64, _path_len: u64) -> u64 {
    ENOSYS
}

/// `SYS_UNLINK(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_unlink(_path_ptr: u64, _path_len: u64) -> u64 {
    ENOSYS
}

/// `SYS_RENAME(old_ptr, old_len, new_ptr, new_len) → 0`
pub unsafe fn sys_fs_rename(
    _old_ptr: u64,
    _old_len: u64,
    _new_ptr: u64,
    _new_len: u64,
) -> u64 {
    ENOSYS
}

/// `SYS_TRUNCATE(fd, new_size) → 0`
pub unsafe fn sys_fs_truncate(_fd: u64, _new_size: u64) -> u64 {
    ENOSYS
}

/// `SYS_SYNC() → 0`
pub unsafe fn sys_fs_sync() -> u64 {
    ENOSYS
}

/// `SYS_SNAPSHOT(name_ptr, name_len) → snapshot_id`
pub unsafe fn sys_fs_snapshot(_name_ptr: u64, _name_len: u64) -> u64 {
    ENOSYS
}

/// `SYS_VERSIONS(path_ptr, path_len, buf_ptr, max) → count`
pub unsafe fn sys_fs_versions(
    _path_ptr: u64,
    _path_len: u64,
    _buf: u64,
    _max: u64,
) -> u64 {
    ENOSYS
}
