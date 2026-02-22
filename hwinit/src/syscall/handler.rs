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

use crate::process::scheduler::{exit_process, SCHEDULER};
use crate::process::signals::Signal;
use crate::serial::puts;

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
// SYS_WRITE — fd-aware write (serial for fd 1/2, VFS for fd >= 3)
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_WRITE(fd, ptr, len)` — write bytes.
///
/// fd 1 (stdout) / fd 2 (stderr): serial console output.
/// fd >= 3: VFS file write.
pub unsafe fn sys_write(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    match fd {
        1 | 2 => {
            let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            if let Ok(s) = core::str::from_utf8(bytes) {
                puts(s);
                len
            } else {
                // Write raw bytes to serial one at a time.
                for &b in bytes { crate::serial::putc(b); }
                len
            }
        }
        fd if fd >= 3 => {
            let fs = match morpheus_helix::vfs::global::fs_global_mut() {
                Some(fs) => fs,
                None => return ENOSYS,
            };
            let fd_table = SCHEDULER.current_fd_table_mut();
            let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            let ts = crate::cpu::tsc::read_tsc();
            match morpheus_helix::vfs::vfs_write(
                &mut fs.device, &mut fs.mount_table, fd_table,
                fd as usize, data, ts,
            ) {
                Ok(n) => n as u64,
                Err(e) => helix_err_to_errno(e),
            }
        }
        _ => EBADF,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_READ — fd-aware read (VFS for fd >= 3)
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_READ(fd, ptr, len)` — read bytes.
///
/// fd 0 (stdin): not yet implemented.
/// fd >= 3: VFS file read.
pub unsafe fn sys_read(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    match fd {
        0 => {
            // stdin — read from kernel keyboard ring buffer.
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
            crate::stdin::read(buf) as u64
        }
        fd if fd >= 3 => {
            let fs = match morpheus_helix::vfs::global::fs_global_mut() {
                Some(fs) => fs,
                None => return ENOSYS,
            };
            let fd_table = SCHEDULER.current_fd_table_mut();
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
            match morpheus_helix::vfs::vfs_read(
                &mut fs.device, &fs.mount_table, fd_table,
                fd as usize, buf,
            ) {
                Ok(n) => n as u64,
                Err(e) => helix_err_to_errno(e),
            }
        }
        _ => EBADF,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_YIELD — voluntary context switch
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_YIELD()` — yield the rest of the current time slice.
///
/// Enables interrupts and executes HLT to relinquish the CPU until the
/// next timer tick.  The timer ISR will context-switch to another Ready
/// process; when we are eventually scheduled again we resume here.
pub unsafe fn sys_yield() -> u64 {
    // STI + HLT is atomic on x86-64: no interrupt window between them.
    // After the timer ISR context-switches away and later resumes us,
    // execution continues at HLT+1.  Re-disable for the syscall return path.
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
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
// SYS_WAIT — wait for a child process to exit
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_WAIT(pid)` — block until child `pid` exits, then return its exit code.
///
/// If the child is already a Zombie, reaps immediately.
/// If `pid` is not a child of the caller, returns -ESRCH.
pub unsafe fn sys_wait(pid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(pid as u32)
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SLEEP — sleep for N milliseconds
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_SLEEP(millis)` — suspend the calling process for at least `millis` ms.
///
/// Computes a TSC deadline and blocks with `BlockReason::Sleep(deadline)`.
/// The scheduler unblocks the process once the deadline has passed.
pub unsafe fn sys_sleep(millis: u64) -> u64 {
    if millis == 0 {
        return 0;
    }
    let tsc_freq = crate::process::scheduler::tsc_frequency();
    if tsc_freq == 0 {
        // TSC not calibrated — cannot compute deadline; return success anyway.
        return 0;
    }
    let ticks_per_ms = tsc_freq / 1000;
    let deadline = crate::cpu::tsc::read_tsc()
        .saturating_add(millis.saturating_mul(ticks_per_ms));
    crate::process::scheduler::block_sleep(deadline)
}

// ═══════════════════════════════════════════════════════════════════════════
// HelixFS Syscall Implementations
// ═══════════════════════════════════════════════════════════════════════════

const ENOSYS: u64 = u64::MAX - 37;
const EINVAL: u64 = u64::MAX;
const ENOENT: u64 = u64::MAX - 2;
const EIO:    u64 = u64::MAX - 5;
const EBADF:  u64 = u64::MAX - 9;

/// Extract a path `&str` from a user pointer+length, with validation.
unsafe fn user_path(ptr: u64, len: u64) -> Option<&'static str> {
    if ptr == 0 || len == 0 || len > 255 { return None; }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    core::str::from_utf8(bytes).ok()
}

fn helix_err_to_errno(_e: morpheus_helix::error::HelixError) -> u64 {
    use morpheus_helix::error::HelixError::*;
    match _e {
        NotFound          => ENOENT,
        AlreadyExists     => u64::MAX - 17,   // EEXIST
        InvalidFd         => EBADF,
        TooManyOpenFiles  => u64::MAX - 24,   // EMFILE
        ReadOnly          => u64::MAX - 30,   // EROFS
        IsADirectory      => u64::MAX - 21,   // EISDIR
        DirectoryNotEmpty => u64::MAX - 39,   // ENOTEMPTY
        NoSpace           => u64::MAX - 28,   // ENOSPC
        MountNotFound     => ENOENT,
        PermissionDenied  => u64::MAX - 13,   // EACCES
        InvalidOffset     => EINVAL,
        IoReadFailed | IoWriteFailed | IoFlushFailed => EIO,
        _                 => EINVAL,
    }
}

/// `SYS_OPEN(path_ptr, path_len, flags) → fd`
pub unsafe fn sys_fs_open(path_ptr: u64, path_len: u64, flags: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    match morpheus_helix::vfs::vfs_open(
        &mut fs.device, &mut fs.mount_table, fd_table,
        path, flags as u32, ts,
    ) {
        Ok(fd) => fd as u64,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_CLOSE(fd) → 0`
pub unsafe fn sys_fs_close(fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    match morpheus_helix::vfs::vfs_close(fd_table, fd as usize) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_SEEK(fd, offset, whence) → new_offset`
pub unsafe fn sys_fs_seek(fd: u64, offset: u64, whence: u64) -> u64 {
    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let fd_table = SCHEDULER.current_fd_table_mut();
    match morpheus_helix::vfs::vfs_seek(&fs.mount_table, fd_table, fd as usize, offset as i64, whence) {
        Ok(pos) => pos,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_STAT(path_ptr, path_len, stat_buf_ptr) → 0`
pub unsafe fn sys_fs_stat(path_ptr: u64, path_len: u64, stat_buf: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(stat) => {
            if stat_buf != 0 {
                let dst = stat_buf as *mut morpheus_helix::types::FileStat;
                *dst = stat;
            }
            0
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_READDIR(path_ptr, path_len, buf_ptr) → count`
pub unsafe fn sys_fs_readdir(path_ptr: u64, path_len: u64, buf_ptr: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    match morpheus_helix::vfs::vfs_readdir(&fs.mount_table, path) {
        Ok(entries) => {
            let count = entries.len();
            if buf_ptr != 0 && count > 0 {
                let dst = buf_ptr as *mut morpheus_helix::types::DirEntry;
                for (i, entry) in entries.iter().enumerate() {
                    *dst.add(i) = *entry;
                }
            }
            count as u64
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_MKDIR(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_mkdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_UNLINK(path_ptr, path_len) → 0`
pub unsafe fn sys_fs_unlink(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, path, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_RENAME(old_ptr, old_len, new_ptr, new_len) → 0`
pub unsafe fn sys_fs_rename(old_ptr: u64, old_len: u64, new_ptr: u64, new_len: u64) -> u64 {
    let old = match user_path(old_ptr, old_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let new = match user_path(new_ptr, new_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let ts = crate::cpu::tsc::read_tsc();
    match morpheus_helix::vfs::vfs_rename(&mut fs.mount_table, old, new, ts) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_TRUNCATE(fd, new_size) → 0` (stub — requires VFS truncate support)
pub unsafe fn sys_fs_truncate(_fd: u64, _new_size: u64) -> u64 {
    ENOSYS
}

/// `SYS_SYNC() → 0`
pub unsafe fn sys_fs_sync() -> u64 {
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    match morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
        Ok(()) => 0,
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_SNAPSHOT(name_ptr, name_len) → snapshot_id` (stub)
pub unsafe fn sys_fs_snapshot(_name_ptr: u64, _name_len: u64) -> u64 {
    ENOSYS
}

/// `SYS_VERSIONS(path_ptr, path_len, buf_ptr, max) → count` (stub)
pub unsafe fn sys_fs_versions(_path_ptr: u64, _path_len: u64, _buf: u64, _max: u64) -> u64 {
    ENOSYS
}
