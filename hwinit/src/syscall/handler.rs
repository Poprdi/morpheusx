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
                for &b in bytes {
                    crate::serial::putc(b);
                }
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
                &mut fs.device,
                &mut fs.mount_table,
                fd_table,
                fd as usize,
                data,
                ts,
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
                &mut fs.device,
                &fs.mount_table,
                fd_table,
                fd as usize,
                buf,
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
    let deadline = crate::cpu::tsc::read_tsc().saturating_add(millis.saturating_mul(ticks_per_ms));
    crate::process::scheduler::block_sleep(deadline)
}

// ═══════════════════════════════════════════════════════════════════════════
// HelixFS Syscall Implementations
// ═══════════════════════════════════════════════════════════════════════════

const ENOSYS: u64 = u64::MAX - 37;
const EINVAL: u64 = u64::MAX;
const ENOENT: u64 = u64::MAX - 2;
const EIO: u64 = u64::MAX - 5;
const EBADF: u64 = u64::MAX - 9;
const ENOMEM: u64 = u64::MAX - 12;
const EFAULT: u64 = u64::MAX - 14;

// ═══════════════════════════════════════════════════════════════════════════
// USER-POINTER VALIDATION
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum canonical user virtual address (lower-half).
const USER_ADDR_LIMIT: u64 = 0x0000_8000_0000_0000;

/// Validate a user pointer + length.
///
/// Returns `true` if the range `[ptr .. ptr+len)` is entirely in the
/// user-accessible lower half of the canonical address space and does
/// not wrap around.  Returns `false` (and the syscall should return
/// `-EFAULT`) otherwise.
#[inline]
fn validate_user_buf(ptr: u64, len: u64) -> bool {
    if ptr == 0 || len == 0 {
        return false;
    }
    let end = ptr.checked_add(len);
    match end {
        Some(e) => e <= USER_ADDR_LIMIT,
        None => false, // overflow
    }
}

/// Extract a path `&str` from a user pointer+length, with validation.
unsafe fn user_path(ptr: u64, len: u64) -> Option<&'static str> {
    if ptr == 0 || len == 0 || len > 255 {
        return None;
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    core::str::from_utf8(bytes).ok()
}

fn helix_err_to_errno(_e: morpheus_helix::error::HelixError) -> u64 {
    use morpheus_helix::error::HelixError::*;
    match _e {
        NotFound => ENOENT,
        AlreadyExists => u64::MAX - 17, // EEXIST
        InvalidFd => EBADF,
        TooManyOpenFiles => u64::MAX - 24,  // EMFILE
        ReadOnly => u64::MAX - 30,          // EROFS
        IsADirectory => u64::MAX - 21,      // EISDIR
        DirectoryNotEmpty => u64::MAX - 39, // ENOTEMPTY
        NoSpace => u64::MAX - 28,           // ENOSPC
        MountNotFound => ENOENT,
        PermissionDenied => u64::MAX - 13, // EACCES
        InvalidOffset => EINVAL,
        IoReadFailed | IoWriteFailed | IoFlushFailed => EIO,
        _ => EINVAL,
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
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags as u32,
        ts,
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
    match morpheus_helix::vfs::vfs_seek(
        &fs.mount_table,
        fd_table,
        fd as usize,
        offset as i64,
        whence,
    ) {
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

/// `SYS_TRUNCATE(path_ptr, path_len, new_size) → 0`
///
/// Truncate the file at `path` to `new_size` bytes.
/// Currently implemented as: open with O_WRITE|O_TRUNC, close.
/// This effectively truncates to 0, then writes nothing — the file becomes empty.
/// A proper VFS truncate(new_size) would need HelixFS support.
pub unsafe fn sys_fs_truncate(path_ptr: u64, path_len: u64, new_size: u64) -> u64 {
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

    // O_WRITE | O_CREATE | O_TRUNC
    let flags: u32 = 0x02 | 0x04 | 0x10;
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    // If new_size > 0, we cannot truly extend/truncate to arbitrary size
    // without VFS support — for now, close immediately (file is truncated to 0).
    let _ = new_size; // TODO: write zeros if new_size > 0 once VFS supports it

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    0
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

/// `SYS_SNAPSHOT(name_ptr, name_len) → snapshot_id`
///
/// Create a filesystem checkpoint. Currently implemented as a full VFS sync
/// with the TSC value returned as the checkpoint identifier.
/// Future: integrate with HelixFS log-structured snapshots.
pub unsafe fn sys_fs_snapshot(name_ptr: u64, name_len: u64) -> u64 {
    // Validate name (optional, for labeling the snapshot).
    let _name = user_path(name_ptr, name_len);

    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    // Sync all dirty data to disk.
    if let Err(e) = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
        return helix_err_to_errno(e);
    }

    // Return TSC as the snapshot ID / checkpoint marker.
    crate::cpu::tsc::read_tsc()
}

/// `SYS_VERSIONS(path_ptr, path_len, buf_ptr, max) → count`
///
/// List version history of a file. Each version entry is a `FileVersion`
/// struct (24 bytes): { lsn: u64, size: u64, op: u32, _pad: u32 }.
///
/// If `buf_ptr` is 0 or `max` is 0, returns the total number of versions.
/// HelixFS supports log-structured versioning via `ops::read::list_versions`.
pub unsafe fn sys_fs_versions(path_ptr: u64, path_len: u64, buf_ptr: u64, max: u64) -> u64 {
    let _path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // HelixFS VFS layer doesn't expose list_versions() yet.
    // The lower-level helix ops::read::list_versions() exists but requires
    // direct block_io + log access which bypasses the VFS mount table.
    // TODO: wire through VFS once vfs_versions() is implemented.
    if buf_ptr == 0 || max == 0 {
        return 0; // No versions available through VFS yet
    }

    0 // 0 versions written
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_CLOCK — monotonic nanoseconds since boot (TSC-based)
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_CLOCK() → nanoseconds`
///
/// Returns monotonic nanoseconds since boot derived from the TSC.
/// If the TSC has not been calibrated, returns 0.
pub unsafe fn sys_clock() -> u64 {
    let freq = crate::process::scheduler::tsc_frequency();
    if freq == 0 {
        return 0;
    }
    let tsc = crate::cpu::tsc::read_tsc();
    // nanos = tsc * 1_000_000_000 / freq
    // Use 128-bit intermediate to avoid overflow.
    let nanos_wide = (tsc as u128) * 1_000_000_000u128 / (freq as u128);
    nanos_wide as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SYSINFO — fill a SysInfo struct for the caller
// ═══════════════════════════════════════════════════════════════════════════

/// `#[repr(C)]` layout shared between kernel and userspace.
/// Must match `libmorpheus::sys::SysInfo` exactly.
#[repr(C)]
pub struct SysInfo {
    pub total_mem: u64,
    pub free_mem: u64,
    pub num_procs: u32,
    pub _pad0: u32,
    pub uptime_ticks: u64,
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
}

/// `SYS_SYSINFO(buf_ptr) → 0`
///
/// Fills `buf_ptr` with a `SysInfo` struct.
pub unsafe fn sys_sysinfo(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<SysInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }

    let registry = crate::memory::global_registry();
    let tsc_freq = crate::process::scheduler::tsc_frequency();
    let (heap_total, heap_used, heap_free) = crate::heap::heap_stats().unwrap_or((0, 0, 0));

    let info = SysInfo {
        total_mem: registry.total_memory(),
        free_mem: registry.free_memory(),
        num_procs: SCHEDULER.live_count(),
        _pad0: 0,
        uptime_ticks: crate::cpu::tsc::read_tsc(),
        tsc_freq,
        heap_total: heap_total as u64,
        heap_used: heap_used as u64,
        heap_free: heap_free as u64,
    };

    let dst = buf_ptr as *mut SysInfo;
    core::ptr::write(dst, info);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_GETPPID — parent process ID
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_GETPPID() → parent_pid`
pub unsafe fn sys_getppid() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    proc.parent_pid as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SPAWN — spawn a child process from an ELF path in the VFS
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_SPAWN(path_ptr, path_len) → child_pid`
///
/// Reads an ELF binary from the filesystem, loads it, and spawns a new
/// user process.  Returns the child PID on success.
pub unsafe fn sys_spawn(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Open the file.
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        0x01, // O_READ
        ts,
    ) {
        Ok(fd) => fd,
        Err(_) => return ENOENT,
    };

    // Stat to get size.
    let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(s) => s,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return EIO;
        }
    };

    let file_size = stat.size as usize;
    if file_size == 0 || file_size > 4 * 1024 * 1024 {
        // Refuse files > 4 MiB for safety.
        let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
        return EINVAL;
    }

    // Allocate physical pages for a temporary read buffer.
    let pages_needed = ((file_size + 4095) / 4096) as u64;
    let registry = crate::memory::global_registry_mut();
    let buf_phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages_needed,
    ) {
        Ok(addr) => addr,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return ENOMEM;
        }
    };

    // Read entire file into the buffer.
    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, file_size);
    let bytes_read = match morpheus_helix::vfs::vfs_read(
        &mut fs.device,
        &fs.mount_table,
        fd_table,
        fd,
        buf,
    ) {
        Ok(n) => n,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            let _ = registry.free_pages(buf_phys, pages_needed);
            return EIO;
        }
    };

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);

    // Extract a short name from the path for the process table.
    let name = path.rsplit('/').next().unwrap_or(path);

    // Spawn the process.
    let elf_data = &buf[..bytes_read];
    let result = crate::process::scheduler::spawn_user_process(name, elf_data);

    // Free the temporary buffer.
    let _ = registry.free_pages(buf_phys, pages_needed);

    match result {
        Ok(pid) => pid as u64,
        Err(_) => ENOMEM,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_MMAP — allocate + map pages into user virtual address space
// ═══════════════════════════════════════════════════════════════════════════

/// Starting virtual address for user mmap allocations.
const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// `SYS_MMAP(pages) → virt_addr`
///
/// Allocates physical pages from MemoryRegistry, maps them into the
/// calling process's address space at the next available virtual address,
/// and returns that virtual address.
pub unsafe fn sys_mmap(pages: u64) -> u64 {
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }

    let proc = SCHEDULER.current_process_mut();

    // Initialize mmap_brk on first call.
    if proc.mmap_brk == 0 {
        proc.mmap_brk = USER_MMAP_BASE;
    }

    let vaddr = proc.mmap_brk;

    // Allocate physical pages.
    let registry = crate::memory::global_registry_mut();
    let phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages,
    ) {
        Ok(addr) => addr,
        Err(_) => return ENOMEM,
    };

    // Map each page into the process address space.
    let flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::WRITABLE)
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, flags).is_err() {
            // On failure, free what we allocated and return error.
            let _ = registry.free_pages(phys, pages);
            return ENOMEM;
        }
    }

    // Zero the memory (important for security — don't leak kernel data).
    core::ptr::write_bytes(phys as *mut u8, 0, (pages * 4096) as usize);

    proc.mmap_brk = vaddr + pages * 4096;
    proc.pages_allocated += pages;

    vaddr
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_MUNMAP — unmap pages from user virtual address space
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_MUNMAP(vaddr, pages) → 0`
///
/// Unmaps pages from the calling process's address space.
/// Currently does not reclaim physical memory (no reverse mapping yet).
pub unsafe fn sys_munmap(vaddr: u64, pages: u64) -> u64 {
    if vaddr == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    // Ensure the address is page-aligned and in user space.
    if vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    let proc = SCHEDULER.current_process_mut();

    // Walk the page table and unmap each page.
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        // Use the kernel unmap function if available.  For now, zero the PTE.
        let _ = crate::paging::kunmap_4k(page_virt);
    }

    if proc.pages_allocated >= pages {
        proc.pages_allocated -= pages;
    }

    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_DUP — duplicate a file descriptor
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_DUP(old_fd) → new_fd`
pub unsafe fn sys_dup(old_fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();

    // Validate old_fd is open.
    let src = match fd_table.get(old_fd as usize) {
        Ok(desc) => *desc,
        Err(_) => return EBADF,
    };

    // Allocate new fd.
    let new_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => return ENOMEM,
    };

    fd_table.fds[new_fd] = src;
    new_fd as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SYSLOG — write to kernel serial log
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_SYSLOG(ptr, len) → len`
///
/// Writes a message directly to the kernel serial log (bypassing the
/// console/window system).  Useful for debugging.
pub unsafe fn sys_syslog(ptr: u64, len: u64) -> u64 {
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }
    if len > (1 << 20) {
        return EINVAL;
    }
    let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
    if let Ok(s) = core::str::from_utf8(bytes) {
        puts("[USR] ");
        puts(s);
        if !s.ends_with('\n') {
            puts("\n");
        }
    } else {
        // Non-UTF8: write raw bytes.
        for &b in bytes {
            crate::serial::putc(b);
        }
    }
    len
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_GETCWD — get current working directory
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_GETCWD(buf_ptr, buf_len) → cwd_len`
///
/// Copies the current working directory into the user buffer.
/// Returns the length of the CWD string.
pub unsafe fn sys_getcwd(buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }
    let proc = SCHEDULER.current_process_mut();
    let cwd = proc.cwd_str();
    let copy_len = cwd.len().min(buf_len as usize);
    let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
    dst.copy_from_slice(&cwd.as_bytes()[..copy_len]);
    cwd.len() as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_CHDIR — change current working directory
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_CHDIR(path_ptr, path_len) → 0`
///
/// Changes the calling process's working directory to the given path.
/// Returns `-ENOENT` if the path does not exist in the VFS.
pub unsafe fn sys_chdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Verify path exists via VFS stat.
    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(_stat) => {
            // TODO: verify it's a directory once FileStat exposes type.
            let proc = SCHEDULER.current_process_mut();
            proc.set_cwd(path);
            0
        }
        Err(_) => ENOENT,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PERSISTENCE — Key-Value store backed by HelixFS /persist/ directory
// ═══════════════════════════════════════════════════════════════════════════
//
// The persistent KV store maps keys to files under `/persist/<key>`.
// Backend is HelixFS today, but the `morpheus-persistent` crate's
// `PersistenceBackend` trait allows swapping to ESP/TPM/NVRAM later.
//
// This gives userland apps a dead-simple "survive reboots" mechanism:
//   persist_put("settings", &config_bytes);
//   persist_get("settings", &mut buf);

/// Persistence subsystem info.
/// Must match `libmorpheus::persist::PersistInfo` exactly.
#[repr(C)]
pub struct PersistInfo {
    /// Bitmask of active backends: bit 0 = HelixFS
    pub backend_flags: u32,
    pub _pad0: u32,
    /// Number of keys currently stored
    pub num_keys: u64,
    /// Total bytes used by values
    pub used_bytes: u64,
}

/// Binary format info returned by `SYS_PE_INFO`.
/// Must match `libmorpheus::persist::BinaryInfo` exactly.
#[repr(C)]
pub struct BinaryInfo {
    /// Format: 0=unknown, 1=ELF64, 2=PE32+
    pub format: u32,
    /// Architecture: 0=unknown, 1=x86_64, 2=aarch64, 3=arm
    pub arch: u32,
    /// Entry point address (RVA for PE, virtual for ELF)
    pub entry_point: u64,
    /// PE ImageBase (0 for ELF)
    pub image_base: u64,
    /// Total file size in bytes
    pub image_size: u64,
    /// Number of sections (PE) or program headers (ELF)
    pub num_sections: u32,
    pub _pad0: u32,
}

/// Build `/persist/<key>` path in a stack buffer.
/// Returns the path as `&str`, or `None` if the key is invalid.
///
/// Keys must be 1-255 bytes, no `/` or `\0`.
unsafe fn persist_path<'a>(key: &str, buf: &'a mut [u8; 272]) -> Option<&'a str> {
    const PREFIX: &[u8] = b"/persist/";
    if key.is_empty() || key.len() > 255 || key.contains('/') || key.contains('\0') {
        return None;
    }
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    buf[PREFIX.len()..PREFIX.len() + key.len()].copy_from_slice(key.as_bytes());
    core::str::from_utf8(&buf[..PREFIX.len() + key.len()]).ok()
}

/// Ensure the `/persist` directory exists. Idempotent — ignores AlreadyExists.
unsafe fn ensure_persist_dir() {
    if let Some(fs) = morpheus_helix::vfs::global::fs_global_mut() {
        let ts = crate::cpu::tsc::read_tsc();
        let _ = morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, "/persist", ts);
    }
}

/// `SYS_PERSIST_PUT(key_ptr, key_len, data_ptr, data_len) → 0`
///
/// Store a named blob to persistent storage (`/persist/<key>`).
/// Max key: 255 bytes (no `/` or NUL). Max value: 4 MiB.
/// Overwrites if key already exists. Data is fsynced to disk.
pub unsafe fn sys_persist_put(key_ptr: u64, key_len: u64, data_ptr: u64, data_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }
    if data_len > 0 && !validate_user_buf(data_ptr, data_len) {
        return EFAULT;
    }
    if data_len > 4 * 1024 * 1024 {
        return EINVAL;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    ensure_persist_dir();

    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    // O_WRITE | O_CREATE | O_TRUNC
    let flags: u32 = 0x02 | 0x04 | 0x10;
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        flags,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    if data_len > 0 {
        let data = core::slice::from_raw_parts(data_ptr as *const u8, data_len as usize);
        if let Err(e) = morpheus_helix::vfs::vfs_write(
            &mut fs.device,
            &mut fs.mount_table,
            fd_table,
            fd,
            data,
            ts,
        ) {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return helix_err_to_errno(e);
        }
    }

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
    0
}

/// `SYS_PERSIST_GET(key_ptr, key_len, buf_ptr, buf_len) → bytes_read`
///
/// Load a named blob from persistent storage.
/// If `buf_len` is 0, returns the value's size without reading.
/// Returns `-ENOENT` if the key doesn't exist.
pub unsafe fn sys_persist_get(key_ptr: u64, key_len: u64, buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }
    if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    // buf_len == 0 → just return file size (stat only).
    if buf_len == 0 {
        return match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
            Ok(stat) => stat.size,
            Err(e) => helix_err_to_errno(e),
        };
    }

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        0x01, // O_READ
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => return helix_err_to_errno(e),
    };

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize);
    let bytes = match morpheus_helix::vfs::vfs_read(
        &mut fs.device,
        &fs.mount_table,
        fd_table,
        fd,
        buf,
    ) {
        Ok(n) => n as u64,
        Err(e) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return helix_err_to_errno(e);
        }
    };

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
    bytes
}

/// `SYS_PERSIST_DEL(key_ptr, key_len) → 0`
///
/// Delete a key from persistent storage. Fsynced to disk.
pub unsafe fn sys_persist_del(key_ptr: u64, key_len: u64) -> u64 {
    if !validate_user_buf(key_ptr, key_len) {
        return EFAULT;
    }

    let key = match user_path(key_ptr, key_len) {
        Some(k) => k,
        None => return EINVAL,
    };

    let mut path_buf = [0u8; 272];
    let path = match persist_path(key, &mut path_buf) {
        Some(p) => p,
        None => return EINVAL,
    };

    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let ts = crate::cpu::tsc::read_tsc();

    match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, path, ts) {
        Ok(()) => {
            let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
            0
        }
        Err(e) => helix_err_to_errno(e),
    }
}

/// `SYS_PERSIST_LIST(buf_ptr, buf_len, offset) → count`
///
/// List keys in persistent storage. Writes NUL-separated key names
/// into `buf_ptr`. Returns the number of keys written. Pass `offset`
/// to skip that many entries (for pagination).
///
/// If `buf_len` is 0, returns the total number of keys.
pub unsafe fn sys_persist_list(buf_ptr: u64, buf_len: u64, offset: u64) -> u64 {
    if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    let entries = match morpheus_helix::vfs::vfs_readdir(&fs.mount_table, "/persist") {
        Ok(e) => e,
        Err(_) => return 0, // directory doesn't exist → 0 keys
    };

    // Filter out "." and ".." and count real entries.
    let real_count = entries
        .iter()
        .filter(|e| {
            let n = &e.name[..e.name_len as usize];
            n != b"." && n != b".."
        })
        .count();

    if buf_len == 0 || buf_ptr == 0 {
        return real_count as u64;
    }

    let buf = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len as usize);
    let mut pos = 0usize;
    let mut count = 0u64;
    let mut skipped = 0u64;

    for entry in entries.iter() {
        let name_bytes = &entry.name[..entry.name_len as usize];
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        let need = name_bytes.len() + 1; // name + NUL terminator
        if pos + need > buf.len() {
            break; // buffer full
        }
        buf[pos..pos + name_bytes.len()].copy_from_slice(name_bytes);
        buf[pos + name_bytes.len()] = 0;
        pos += need;
        count += 1;
    }

    count
}

/// `SYS_PERSIST_INFO(info_ptr) → 0`
///
/// Fill a `PersistInfo` struct with backend status and usage statistics.
pub unsafe fn sys_persist_info(info_ptr: u64) -> u64 {
    let size = core::mem::size_of::<PersistInfo>() as u64;
    if !validate_user_buf(info_ptr, size) {
        return EFAULT;
    }

    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    let mut num_keys = 0u64;
    let mut used_bytes = 0u64;

    if let Ok(entries) = morpheus_helix::vfs::vfs_readdir(&fs.mount_table, "/persist") {
        for entry in entries.iter() {
            let name_bytes = &entry.name[..entry.name_len as usize];
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            // Build path to stat each file.
            let mut path_buf = [0u8; 272];
            let prefix = b"/persist/";
            if name_bytes.len() > 255 {
                continue;
            }
            path_buf[..prefix.len()].copy_from_slice(prefix);
            path_buf[prefix.len()..prefix.len() + name_bytes.len()]
                .copy_from_slice(name_bytes);
            if let Ok(p) =
                core::str::from_utf8(&path_buf[..prefix.len() + name_bytes.len()])
            {
                if let Ok(stat) = morpheus_helix::vfs::vfs_stat(&fs.mount_table, p) {
                    num_keys += 1;
                    used_bytes += stat.size;
                }
            }
        }
    }

    let info = PersistInfo {
        backend_flags: 1, // bit 0 = HelixFS active
        _pad0: 0,
        num_keys,
        used_bytes,
    };

    core::ptr::write(info_ptr as *mut PersistInfo, info);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PE_INFO — Binary introspection (PE + ELF)
// ═══════════════════════════════════════════════════════════════════════════
//
// Uses `morpheus_persistent::pe::header::PeHeaders` for PE/COFF parsing
// and inline ELF64 header parsing for ELF binaries.

/// `SYS_PE_INFO(path_ptr, path_len, info_ptr) → 0`
///
/// Read a binary file from the VFS, detect its format (PE32+ or ELF64),
/// parse the headers, and fill a `BinaryInfo` struct.
///
/// Max file read for headers: 64 KiB.
pub unsafe fn sys_pe_info(path_ptr: u64, path_len: u64, info_ptr: u64) -> u64 {
    let info_size = core::mem::size_of::<BinaryInfo>() as u64;
    if !validate_user_buf(info_ptr, info_size) {
        return EFAULT;
    }

    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };

    // Stat to get file size.
    let file_size = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(s) => s.size as usize,
        Err(e) => return helix_err_to_errno(e),
    };

    if file_size < 64 {
        return EINVAL; // too small to be any known binary format
    }

    // Read at most 64 KB for header parsing.
    let read_size = file_size.min(65536);
    let pages_needed = ((read_size + 4095) / 4096) as u64;

    let registry = crate::memory::global_registry_mut();
    let buf_phys = match registry.allocate_pages(
        crate::memory::AllocateType::AnyPages,
        crate::memory::MemoryType::Allocated,
        pages_needed,
    ) {
        Ok(addr) => addr,
        Err(_) => return ENOMEM,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = crate::cpu::tsc::read_tsc();

    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        fd_table,
        path,
        0x01,
        ts,
    ) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = registry.free_pages(buf_phys, pages_needed);
            return helix_err_to_errno(e);
        }
    };

    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, read_size);
    let bytes_read = match morpheus_helix::vfs::vfs_read(
        &mut fs.device,
        &fs.mount_table,
        fd_table,
        fd,
        buf,
    ) {
        Ok(n) => n,
        Err(e) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            let _ = registry.free_pages(buf_phys, pages_needed);
            return helix_err_to_errno(e);
        }
    };
    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);

    let data = core::slice::from_raw_parts(buf_phys as *const u8, bytes_read);

    let mut info = BinaryInfo {
        format: 0,
        arch: 0,
        entry_point: 0,
        image_base: 0,
        image_size: file_size as u64,
        num_sections: 0,
        _pad0: 0,
    };

    // ── Detect ELF ──────────────────────────────────────────────────
    if bytes_read >= 64
        && data[0] == 0x7f
        && data[1] == b'E'
        && data[2] == b'L'
        && data[3] == b'F'
    {
        info.format = 1; // ELF64
        let ei_class = data[4];
        if ei_class == 2 {
            // 64-bit
            let e_machine = u16::from_le_bytes([data[18], data[19]]);
            info.arch = match e_machine {
                0x3E => 1, // EM_X86_64
                0xB7 => 2, // EM_AARCH64
                0x28 => 3, // EM_ARM
                _ => 0,
            };
            info.entry_point = u64::from_le_bytes([
                data[24], data[25], data[26], data[27], data[28], data[29], data[30],
                data[31],
            ]);
            info.num_sections = u16::from_le_bytes([data[60], data[61]]) as u32;
        }
    }
    // ── Detect PE/MZ ────────────────────────────────────────────────
    else if bytes_read >= 256 && data[0] == b'M' && data[1] == b'Z' {
        info.format = 2; // PE32+
        // Use morpheus_persistent's PE parser for full header extraction.
        match morpheus_persistent::pe::header::PeHeaders::parse(
            buf_phys as *const u8,
            bytes_read,
        ) {
            Ok(pe) => {
                info.image_base = pe.optional.image_base;
                info.entry_point = pe.optional.address_of_entry_point as u64;
                info.num_sections = pe.coff.number_of_sections as u32;
                match pe.arch() {
                    Ok(morpheus_persistent::pe::PeArch::X64) => info.arch = 1,
                    Ok(morpheus_persistent::pe::PeArch::ARM64) => info.arch = 2,
                    Ok(morpheus_persistent::pe::PeArch::ARM) => info.arch = 3,
                    _ => info.arch = 0,
                }
            }
            Err(_) => {} // format=2 but details left as zero
        }
    }

    let _ = registry.free_pages(buf_phys, pages_needed);

    core::ptr::write(info_ptr as *mut BinaryInfo, info);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// NIC REGISTRATION — function-pointer bridge for network drivers
// ═══════════════════════════════════════════════════════════════════════════
//
// hwinit does not depend on morpheus-network, so the NIC driver is
// registered by the bootloader via function pointers.

/// NIC operations function-pointer table.  The bootloader fills this in
/// after initialising the network driver, before entering the event loop.
#[repr(C)]
pub struct NicOps {
    /// Transmit a raw Ethernet frame.  Returns 0 on success, -1 on error.
    pub tx: Option<unsafe fn(frame: *const u8, len: usize) -> i64>,
    /// Receive a raw Ethernet frame into `buf`.  Returns bytes received, 0 if none.
    pub rx: Option<unsafe fn(buf: *mut u8, buf_len: usize) -> i64>,
    /// Get link status.  Returns 1 if link is up, 0 if down.
    pub link_up: Option<unsafe fn() -> i64>,
    /// Get 6-byte MAC address.  Writes to `out`.  Returns 0 on success.
    pub mac: Option<unsafe fn(out: *mut u8) -> i64>,
    /// Refill RX descriptor ring.
    pub refill: Option<unsafe fn() -> i64>,
}

static mut NIC_OPS: NicOps = NicOps {
    tx: None,
    rx: None,
    link_up: None,
    mac: None,
    refill: None,
};

/// Register NIC function pointers.  Called by the bootloader after driver init.
pub unsafe fn register_nic(ops: NicOps) {
    NIC_OPS = ops;
}

/// NIC info returned by SYS_NIC_INFO.
#[repr(C)]
pub struct NicInfo {
    /// 6-byte MAC address, padded to 8.
    pub mac: [u8; 8],
    /// 1 if link up, 0 if down.
    pub link_up: u32,
    /// 1 if NIC is registered, 0 if not.
    pub present: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// FRAMEBUFFER REGISTRATION — pass FB info from bootloader to hwinit
// ═══════════════════════════════════════════════════════════════════════════

/// Framebuffer information registered by the bootloader.
/// Matches display/src/types.rs FramebufferInfo layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FbInfo {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 0 = RGBX, 1 = BGRX
    pub format: u32,
}

static mut FB_REGISTERED: Option<FbInfo> = None;

/// Register framebuffer info.  Called by bootloader before entering desktop.
pub unsafe fn register_framebuffer(info: FbInfo) {
    FB_REGISTERED = Some(info);
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_INFO (32) — get NIC information
// ═══════════════════════════════════════════════════════════════════════════

const ENODEV: u64 = u64::MAX - 19;

/// `SYS_NIC_INFO(buf_ptr) → 0`
pub unsafe fn sys_nic_info(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<NicInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }
    let mut info = NicInfo {
        mac: [0u8; 8],
        link_up: 0,
        present: 0,
    };

    if let Some(mac_fn) = NIC_OPS.mac {
        info.present = 1;
        mac_fn(info.mac.as_mut_ptr());
        if let Some(link_fn) = NIC_OPS.link_up {
            info.link_up = link_fn() as u32;
        }
    }

    core::ptr::write(buf_ptr as *mut NicInfo, info);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_TX (33) — transmit a raw Ethernet frame
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_NIC_TX(frame_ptr, frame_len) → 0`
pub unsafe fn sys_nic_tx(frame_ptr: u64, frame_len: u64) -> u64 {
    if !validate_user_buf(frame_ptr, frame_len) {
        return EFAULT;
    }
    if frame_len < 14 || frame_len > 9000 {
        return EINVAL; // min Ethernet header, max jumbo frame
    }
    match NIC_OPS.tx {
        Some(tx_fn) => {
            let rc = tx_fn(frame_ptr as *const u8, frame_len as usize);
            if rc < 0 { EIO } else { 0 }
        }
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_RX (34) — receive a raw Ethernet frame
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_NIC_RX(buf_ptr, buf_len) → bytes_received`
pub unsafe fn sys_nic_rx(buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }
    match NIC_OPS.rx {
        Some(rx_fn) => {
            let rc = rx_fn(buf_ptr as *mut u8, buf_len as usize);
            if rc < 0 { EIO } else { rc as u64 }
        }
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_LINK (35) — get link status
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_NIC_LINK() → 0/1 (down/up)`
pub unsafe fn sys_nic_link() -> u64 {
    match NIC_OPS.link_up {
        Some(f) => f() as u64,
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_MAC (36) — get 6-byte MAC address
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_NIC_MAC(buf_ptr) → 0`
pub unsafe fn sys_nic_mac(buf_ptr: u64) -> u64 {
    if !validate_user_buf(buf_ptr, 6) {
        return EFAULT;
    }
    match NIC_OPS.mac {
        Some(f) => {
            f(buf_ptr as *mut u8);
            0
        }
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_NIC_REFILL (37) — refill RX descriptor ring
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_NIC_REFILL() → 0`
pub unsafe fn sys_nic_refill() -> u64 {
    match NIC_OPS.refill {
        Some(f) => {
            f();
            0
        }
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_IOCTL (42) — device control
// ═══════════════════════════════════════════════════════════════════════════

// ioctl commands
const IOCTL_FIONREAD: u64 = 0x541B; // bytes available on fd (like FIONREAD)
const IOCTL_TIOCGWINSZ: u64 = 0x5413; // get terminal window size

/// `SYS_IOCTL(fd, cmd, arg) → result`
pub unsafe fn sys_ioctl(fd: u64, cmd: u64, arg: u64) -> u64 {
    match (fd, cmd) {
        // stdin: check if keyboard data is available
        (0, IOCTL_FIONREAD) => {
            let avail = crate::stdin::available();
            if arg != 0 && validate_user_buf(arg, 4) {
                core::ptr::write(arg as *mut u32, avail as u32);
            }
            avail as u64
        }
        // Terminal window size: return 80×25 default
        (0..=2, IOCTL_TIOCGWINSZ) => {
            if arg != 0 && validate_user_buf(arg, 8) {
                let buf = arg as *mut u16;
                *buf = 25;        // ws_row
                *buf.add(1) = 80; // ws_col
                *buf.add(2) = 0;  // ws_xpixel
                *buf.add(3) = 0;  // ws_ypixel
            }
            0
        }
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_MOUNT (43) — mount a filesystem
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_MOUNT(src_ptr, src_len, dst_ptr, dst_len) → 0`
///
/// Mount the HelixFS volume at `src` to the mount point `dst`.
/// Currently a no-op success since HelixFS auto-mounts at `/`.
pub unsafe fn sys_mount(src_ptr: u64, src_len: u64, dst_ptr: u64, dst_len: u64) -> u64 {
    let _src = match user_path(src_ptr, src_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    let _dst = match user_path(dst_ptr, dst_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    // HelixFS is always mounted at "/" — additional mounts not supported yet.
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_UMOUNT (44) — unmount a filesystem
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_UMOUNT(path_ptr, path_len) → 0`
///
/// Unmount the filesystem at `path`.  Syncs dirty data before unmounting.
/// Currently: syncs and returns success (root cannot be truly unmounted).
pub unsafe fn sys_umount(path_ptr: u64, path_len: u64) -> u64 {
    let _path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };
    // Sync all dirty data before "unmounting".
    let fs = match morpheus_helix::vfs::global::fs_global_mut() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    let _ = morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_POLL (45) — poll file descriptors for readiness
// ═══════════════════════════════════════════════════════════════════════════

/// Poll entry (matches POSIX pollfd).
#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;

/// `SYS_POLL(fds_ptr, nfds, timeout_ms) → ready_count`
///
/// Check if file descriptors are ready for I/O.
/// - fd 0 (stdin): POLLIN if keyboard data available.
/// - fd 1/2 (stdout/stderr): always POLLOUT (serial is always writable).
/// - fd >= 3 (VFS): always POLLIN|POLLOUT (files are always ready).
pub unsafe fn sys_poll(fds_ptr: u64, nfds: u64, timeout_ms: u64) -> u64 {
    if nfds == 0 {
        // Just sleep for timeout_ms.
        if timeout_ms > 0 {
            let _ = sys_sleep(timeout_ms);
        }
        return 0;
    }
    let size = nfds * core::mem::size_of::<PollFd>() as u64;
    if !validate_user_buf(fds_ptr, size) {
        return EFAULT;
    }

    let fds = core::slice::from_raw_parts_mut(fds_ptr as *mut PollFd, nfds as usize);
    let mut ready = 0u64;

    for pfd in fds.iter_mut() {
        pfd.revents = 0;
        match pfd.fd {
            0 => {
                // stdin
                if pfd.events & POLLIN != 0 && crate::stdin::available() > 0 {
                    pfd.revents |= POLLIN;
                    ready += 1;
                }
            }
            1 | 2 => {
                // stdout/stderr — serial always writable
                if pfd.events & POLLOUT != 0 {
                    pfd.revents |= POLLOUT;
                    ready += 1;
                }
            }
            fd if fd >= 3 => {
                // VFS files are always "ready"
                if pfd.events & POLLIN != 0 {
                    pfd.revents |= POLLIN;
                }
                if pfd.events & POLLOUT != 0 {
                    pfd.revents |= POLLOUT;
                }
                if pfd.revents != 0 {
                    ready += 1;
                }
            }
            _ => {
                pfd.revents = POLLERR;
                ready += 1;
            }
        }
    }

    // If nothing is ready yet and timeout > 0, sleep and recheck.
    if ready == 0 && timeout_ms > 0 {
        let _ = sys_sleep(timeout_ms.min(100));
        // Re-check stdin after sleeping.
        for pfd in fds.iter_mut() {
            if pfd.fd == 0 && pfd.events & POLLIN != 0 && crate::stdin::available() > 0 {
                pfd.revents |= POLLIN;
                ready += 1;
            }
        }
    }

    ready
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PORT_IN (52) — read from I/O port
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_PORT_IN(port, width) → value`
///
/// Read from an x86 I/O port.  `width` is 1 (byte), 2 (word), or 4 (dword).
pub unsafe fn sys_port_in(port: u64, width: u64) -> u64 {
    if port > 0xFFFF {
        return EINVAL;
    }
    let port = port as u16;
    match width {
        1 => crate::cpu::pio::inb(port) as u64,
        2 => crate::cpu::pio::inw(port) as u64,
        4 => crate::cpu::pio::inl(port) as u64,
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PORT_OUT (53) — write to I/O port
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_PORT_OUT(port, width, value) → 0`
///
/// Write to an x86 I/O port.  `width` is 1, 2, or 4.
pub unsafe fn sys_port_out(port: u64, width: u64, value: u64) -> u64 {
    if port > 0xFFFF {
        return EINVAL;
    }
    let port = port as u16;
    match width {
        1 => { crate::cpu::pio::outb(port, value as u8); 0 }
        2 => { crate::cpu::pio::outw(port, value as u16); 0 }
        4 => { crate::cpu::pio::outl(port, value as u32); 0 }
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PCI_CFG_READ (54) — read PCI configuration space
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_PCI_CFG_READ(bdf, offset, width) → value`
///
/// Read PCI configuration register.
///   bdf = bus << 16 | device << 8 | function
///   offset = register offset (0-255)
///   width = 1, 2, or 4
pub unsafe fn sys_pci_cfg_read(bdf: u64, offset: u64, width: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 255 {
        return EINVAL;
    }
    let addr = crate::pci::PciAddr { bus, device: dev, function: func };
    let off = offset as u8;
    match width {
        1 => crate::pci::pci_cfg_read8(addr, off) as u64,
        2 => crate::pci::pci_cfg_read16(addr, off) as u64,
        4 => crate::pci::pci_cfg_read32(addr, off) as u64,
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PCI_CFG_WRITE (55) — write PCI configuration space
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_PCI_CFG_WRITE(bdf, offset, width, value) → 0`
pub unsafe fn sys_pci_cfg_write(bdf: u64, offset: u64, width: u64, value: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 255 {
        return EINVAL;
    }
    let addr = crate::pci::PciAddr { bus, device: dev, function: func };
    let off = offset as u8;
    match width {
        1 => { crate::pci::pci_cfg_write8(addr, off, value as u8); 0 }
        2 => { crate::pci::pci_cfg_write16(addr, off, value as u16); 0 }
        4 => { crate::pci::pci_cfg_write32(addr, off, value as u32); 0 }
        _ => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_DMA_ALLOC (56) — allocate DMA-safe memory below 4GB
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_DMA_ALLOC(pages) → phys_addr`
///
/// Allocates physically contiguous pages below 4GB, suitable for DMA.
pub unsafe fn sys_dma_alloc(pages: u64) -> u64 {
    if pages == 0 || pages > 512 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }
    let registry = crate::memory::global_registry_mut();
    match registry.alloc_dma_pages(pages) {
        Ok(addr) => {
            // Zero the memory for security.
            core::ptr::write_bytes(addr as *mut u8, 0, (pages * 4096) as usize);
            addr
        }
        Err(_) => ENOMEM,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_DMA_FREE (57) — free DMA memory
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_DMA_FREE(phys, pages) → 0`
pub unsafe fn sys_dma_free(phys: u64, pages: u64) -> u64 {
    if phys == 0 || pages == 0 || pages > 512 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }
    let registry = crate::memory::global_registry_mut();
    match registry.free_pages(phys, pages) {
        Ok(()) => 0,
        Err(_) => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_MAP_PHYS (58) — map physical address into process virtual space
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_MAP_PHYS(phys, pages, flags) → virt_addr`
///
/// Maps `pages` 4K pages starting at physical address `phys` into the
/// calling process's virtual address space.
///
/// Flags: bit 0 = writable, bit 1 = uncacheable.
pub unsafe fn sys_map_phys(phys: u64, pages: u64, flags: u64) -> u64 {
    if phys == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if phys & 0xFFF != 0 {
        return EINVAL; // must be page-aligned
    }

    let proc = SCHEDULER.current_process_mut();
    if proc.mmap_brk == 0 {
        proc.mmap_brk = 0x0000_0040_0000_0000;
    }
    let vaddr = proc.mmap_brk;

    let writable = flags & 1 != 0;
    let uncacheable = flags & 2 != 0;

    let mut pte_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);
    if writable {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if uncacheable {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::CACHE_DISABLE);
    }

    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, pte_flags).is_err() {
            return ENOMEM;
        }
    }

    proc.mmap_brk = vaddr + pages * 4096;
    vaddr
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_VIRT_TO_PHYS (59) — translate virtual to physical address
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_VIRT_TO_PHYS(virt) → phys`
///
/// Walk the page tables to resolve a virtual address to its physical address.
/// Returns EINVAL if the page is not mapped.
pub unsafe fn sys_virt_to_phys(virt: u64) -> u64 {
    match crate::paging::kvirt_to_phys(virt) {
        Some(phys) => phys,
        None => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_IRQ_ATTACH (60) — enable an IRQ line
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_IRQ_ATTACH(irq_num) → 0`
///
/// Enable the specified IRQ line on the PIC.  The caller is responsible
/// for handling interrupts (via polling or shared interrupt mechanism).
pub unsafe fn sys_irq_attach(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    crate::cpu::pic::enable_irq(irq_num as u8);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_IRQ_ACK (61) — acknowledge an IRQ (send EOI)
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_IRQ_ACK(irq_num) → 0`
///
/// Send End-Of-Interrupt for the specified IRQ number.
pub unsafe fn sys_irq_ack(irq_num: u64) -> u64 {
    if irq_num > 15 {
        return EINVAL;
    }
    crate::cpu::pic::send_eoi(irq_num as u8);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_CACHE_FLUSH (62) — flush CPU cache for an address range
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_CACHE_FLUSH(addr, len) → 0`
///
/// Flush cache lines covering `[addr, addr+len)`.  Essential for DMA
/// coherence when the CPU writes data that a device will read.
pub unsafe fn sys_cache_flush(addr: u64, len: u64) -> u64 {
    if addr == 0 || len == 0 {
        return EINVAL;
    }
    if len > 64 * 1024 * 1024 {
        return EINVAL; // cap at 64MB to avoid excessive stalls
    }
    crate::cpu::cache::flush_range(addr as *const u8, len as usize);
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_FB_INFO (63) — get framebuffer information
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_FB_INFO(buf_ptr) → 0`
///
/// Copy the `FbInfo` struct to the user buffer.  Returns -ENODEV if
/// no framebuffer has been registered.
pub unsafe fn sys_fb_info(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<FbInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }
    match FB_REGISTERED {
        Some(info) => {
            core::ptr::write(buf_ptr as *mut FbInfo, info);
            0
        }
        None => ENODEV,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_FB_MAP (64) — map framebuffer into process virtual address space
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_FB_MAP() → virt_addr`
///
/// Maps the physical framebuffer into the calling process's address space
/// as writable + uncacheable (write-combining would be better but requires
/// PAT setup).  Returns the virtual address of the mapped framebuffer.
pub unsafe fn sys_fb_map() -> u64 {
    let info = match FB_REGISTERED {
        Some(i) => i,
        None => return ENODEV,
    };

    let pages = ((info.size as u64) + 4095) / 4096;
    // Use MAP_PHYS with writable + uncacheable flags.
    sys_map_phys(info.base, pages, 0x03) // flags: writable(1) | uncacheable(2)
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_PS (65) — list all processes
// ═══════════════════════════════════════════════════════════════════════════

/// Process info returned by SYS_PS.
/// Must match `libmorpheus::process::PsEntry` exactly.
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,      // 0=Ready, 1=Running, 2=Blocked, 3=Zombie, 4=Terminated
    pub priority: u32,
    pub cpu_ticks: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32],  // NUL-terminated
}

/// `SYS_PS(buf_ptr, max_count) → count`
///
/// List all processes.  Writes up to `max_count` `PsEntry` structs to `buf_ptr`.
pub unsafe fn sys_ps(buf_ptr: u64, max_count: u64) -> u64 {
    if max_count == 0 || buf_ptr == 0 {
        return SCHEDULER.live_count() as u64;
    }
    let entry_size = core::mem::size_of::<PsEntry>() as u64;
    let total_size = max_count.saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total_size) {
        return EFAULT;
    }

    // Use the scheduler's snapshot_processes to get ProcessInfo array.
    let max = max_count.min(64) as usize;
    let mut infos = [crate::process::scheduler::ProcessInfo::zeroed(); 64];
    let count = SCHEDULER.snapshot_processes(&mut infos[..max]);

    let out = core::slice::from_raw_parts_mut(buf_ptr as *mut PsEntry, max);
    for i in 0..count {
        let pi = &infos[i];
        let state_u32 = match pi.state {
            crate::process::ProcessState::Ready => 0,
            crate::process::ProcessState::Running => 1,
            crate::process::ProcessState::Blocked(_) => 2,
            crate::process::ProcessState::Zombie => 3,
            crate::process::ProcessState::Terminated => 4,
        };
        let mut entry = PsEntry {
            pid: pi.pid,
            ppid: 0, // ProcessInfo doesn't carry ppid; 0 is fine
            state: state_u32,
            priority: pi.priority as u32,
            cpu_ticks: pi.cpu_ticks,
            pages_alloc: pi.pages_alloc,
            name: [0u8; 32],
        };
        let name_bytes = pi.name_bytes();
        let copy_len = name_bytes.len().min(31);
        entry.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        out[i] = entry;
    }

    count as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SIGACTION (66) — register a signal handler
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_SIGACTION(signum, handler_addr) → old_handler`
///
/// Register a handler function for the given signal.
/// `handler_addr` = 0 means SIG_DFL (default action).
/// `handler_addr` = 1 means SIG_IGN (ignore).
/// Returns the previous handler address, or EINVAL for invalid signals.
pub unsafe fn sys_sigaction(signum: u64, handler: u64) -> u64 {
    let sig = match Signal::from_u8(signum as u8) {
        Some(s) => s,
        None => return EINVAL,
    };
    // SIGKILL and SIGSTOP cannot be caught or ignored.
    if matches!(sig, Signal::SIGKILL | Signal::SIGSTOP) {
        return EINVAL;
    }

    let proc = SCHEDULER.current_process_mut();

    // Signal handlers stored per-process.  Use pending_signals for now
    // as a simplified mechanism — the real handler addresses are stored
    // in a separate array.  For now, return 0 (SIG_DFL) as old handler
    // and record the new handler address.
    // TODO: add signal_handlers: [u64; 32] field to Process struct.
    let _ = handler;
    let _ = proc;

    // For now, accept the registration and return 0 (old = SIG_DFL).
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_SETPRIORITY (67) — set process scheduling priority
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_SETPRIORITY(pid, priority) → 0`
///
/// Set the scheduling priority of a process.
/// pid = 0 means current process.
/// priority: 0-255 (0 = highest, 255 = lowest).
pub unsafe fn sys_setpriority(pid: u64, priority: u64) -> u64 {
    if priority > 255 {
        return EINVAL;
    }
    let target_pid = if pid == 0 {
        SCHEDULER.current_pid()
    } else {
        pid as u32
    };
    match SCHEDULER.set_priority(target_pid, priority as u8) {
        Ok(()) => 0,
        Err(_) => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_GETPRIORITY (68) — get process scheduling priority
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_GETPRIORITY(pid) → priority`
///
/// Get the scheduling priority.  pid = 0 means current process.
pub unsafe fn sys_getpriority(pid: u64) -> u64 {
    let target_pid = if pid == 0 {
        SCHEDULER.current_pid()
    } else {
        pid as u32
    };
    match SCHEDULER.get_priority(target_pid) {
        Ok(p) => p as u64,
        Err(_) => EINVAL,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_CPUID (69) — execute CPUID instruction
// ═══════════════════════════════════════════════════════════════════════════

/// CPUID result.
#[repr(C)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// `SYS_CPUID(leaf, subleaf, result_ptr) → 0`
///
/// Execute the CPUID instruction with the given leaf/subleaf and write
/// the 4 result registers to `result_ptr`.
pub unsafe fn sys_cpuid(leaf: u64, subleaf: u64, result_ptr: u64) -> u64 {
    let size = core::mem::size_of::<CpuidResult>() as u64;
    if !validate_user_buf(result_ptr, size) {
        return EFAULT;
    }

    let eax_in = leaf as u32;
    let ecx_in = subleaf as u32;
    let eax: u32;
    let ecx: u32;
    let edx: u32;

    let ebx_raw: u64;
    core::arch::asm!(
        "push rbx",
        "cpuid",
        "mov {rbx_out}, rbx",
        "pop rbx",
        rbx_out = lateout(reg) ebx_raw,
        inlateout("eax") eax_in => eax,
        inlateout("ecx") ecx_in => ecx,
        lateout("edx") edx,
        options(nostack, nomem),
    );
    let ebx = ebx_raw as u32;

    core::ptr::write(
        result_ptr as *mut CpuidResult,
        CpuidResult { eax, ebx, ecx, edx },
    );
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_RDTSC (70) — read TSC with frequency info
// ═══════════════════════════════════════════════════════════════════════════

/// TSC result struct.
#[repr(C)]
pub struct TscResult {
    pub tsc: u64,
    pub frequency: u64,
}

/// `SYS_RDTSC(result_ptr) → tsc_value`
///
/// Read the Time Stamp Counter.  If `result_ptr` is non-zero, also
/// writes a `TscResult` struct with both the TSC value and calibrated
/// frequency in Hz.
pub unsafe fn sys_rdtsc(result_ptr: u64) -> u64 {
    let tsc = crate::cpu::tsc::read_tsc();
    let freq = crate::process::scheduler::tsc_frequency();

    if result_ptr != 0 {
        let size = core::mem::size_of::<TscResult>() as u64;
        if validate_user_buf(result_ptr, size) {
            core::ptr::write(
                result_ptr as *mut TscResult,
                TscResult {
                    tsc,
                    frequency: freq,
                },
            );
        }
    }

    tsc
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_BOOT_LOG (71) — read kernel boot log
// ═══════════════════════════════════════════════════════════════════════════

/// `SYS_BOOT_LOG(buf_ptr, buf_len) → bytes_written`
///
/// Copy the kernel boot log (serial output captured during init) into
/// the user buffer.  Returns the number of bytes written.
pub unsafe fn sys_boot_log(buf_ptr: u64, buf_len: u64) -> u64 {
    if buf_len == 0 {
        // Return total log size.
        return crate::serial::boot_log().len() as u64;
    }
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }

    let log = crate::serial::boot_log();
    let copy_len = log.len().min(buf_len as usize);
    let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
    dst.copy_from_slice(&log.as_bytes()[..copy_len]);
    copy_len as u64
}

// ═══════════════════════════════════════════════════════════════════════════
// SYS_MEMMAP (72) — read physical memory map
// ═══════════════════════════════════════════════════════════════════════════

/// Memory map entry returned by SYS_MEMMAP.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemmapEntry {
    pub phys_start: u64,
    pub num_pages: u64,
    pub mem_type: u32,
    pub _pad: u32,
}

/// `SYS_MEMMAP(buf_ptr, max_entries) → count`
///
/// Copy the physical memory map into the user buffer.
/// If `buf_ptr` is 0, returns the total number of entries.
pub unsafe fn sys_memmap(buf_ptr: u64, max_entries: u64) -> u64 {
    let registry = crate::memory::global_registry();
    let (_key, total) = registry.get_memory_map();

    if buf_ptr == 0 || max_entries == 0 {
        return total as u64;
    }

    let entry_size = core::mem::size_of::<MemmapEntry>() as u64;
    let total_size = max_entries.saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total_size) {
        return EFAULT;
    }

    let out = core::slice::from_raw_parts_mut(
        buf_ptr as *mut MemmapEntry,
        max_entries as usize,
    );
    let count = total.min(max_entries as usize);

    for i in 0..count {
        if let Some(desc) = registry.get_descriptor(i) {
            out[i] = MemmapEntry {
                phys_start: desc.physical_start,
                num_pages: desc.number_of_pages,
                mem_type: desc.mem_type as u32,
                _pad: 0,
            };
        } else {
            return i as u64;
        }
    }

    count as u64
}
