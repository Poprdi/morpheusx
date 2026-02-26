//! Syscall handlers. args come in as u64, result goes out as u64.
//! Negative = errno, 0 = ok, positive = data. MS x64 ABI.

use crate::process::scheduler::{exit_process, SCHEDULER};
use crate::process::signals::Signal;
use crate::serial::puts;
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

// SYS_EXIT — terminate the current process

/// `SYS_EXIT(code: i32)` — terminate the calling process.
///
/// Never returns.
pub unsafe fn sys_exit(code: u64) -> u64 {
    exit_process(code as i32);
}

// SYS_WRITE — fd-aware write (serial for fd 1/2, VFS for fd >= 3)

/// `SYS_WRITE(fd, ptr, len)` — write bytes.
///
/// fd 1 (stdout) / fd 2 (stderr): serial console output.
/// fd >= 3: VFS file write.
pub unsafe fn sys_write(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }
    match fd {
        1 | 2 => {
            let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            // Capture output for the desktop shell widget
            crate::stdout::push(bytes);
            if let Ok(s) = core::str::from_utf8(bytes) {
                puts(s);
                len
            } else {
                for &b in bytes {
                    crate::serial::putc(b);
                }
                len
            }
        }
        fd if fd >= 3 => {
            // Check if this fd is a pipe write end.
            let fd_table = SCHEDULER.current_fd_table_mut();
            if let Ok(desc) = fd_table.get(fd as usize) {
                if desc.flags & O_PIPE_WRITE != 0 {
                    let pipe_idx = desc.mount_idx;
                    let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);
                    // Check if anyone is reading the other end.
                    if crate::pipe::pipe_readers(pipe_idx) == 0 {
                        return EPIPE;
                    }
                    let n = crate::pipe::pipe_write(pipe_idx, data);
                    crate::process::scheduler::wake_pipe_readers(pipe_idx);
                    return n as u64;
                }
            }
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

// SYS_READ — fd-aware read (VFS for fd >= 3)

/// `SYS_READ(fd, ptr, len)` — read bytes.
///
/// fd 0 (stdin): not yet implemented.
/// fd >= 3: VFS file read.
pub unsafe fn sys_read(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }
    match fd {
        0 => {
            // stdin — non-blocking read from kernel keyboard ring buffer.
            // Returns 0 immediately if no data is available; callers that
            // need blocking behaviour must poll in their own loop.
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
            crate::stdin::read(buf) as u64
        }
        fd if fd >= 3 => {
            // Check if this fd is actually a pipe read end.
            let fd_table = SCHEDULER.current_fd_table_mut();
            if let Ok(desc) = fd_table.get(fd as usize) {
                if desc.flags & O_PIPE_READ != 0 {
                    let pipe_idx = desc.mount_idx;
                    let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
                    return sys_pipe_read_blocking(pipe_idx, buf);
                }
            }
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

// SYS_YIELD — voluntary context switch

/// Yield. STI+HLT is atomic on x86-64 — no surprise interrupts.
pub unsafe fn sys_yield() -> u64 {
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    0
}

// SYS_GETPID

pub unsafe fn sys_getpid() -> u64 {
    SCHEDULER.current_pid() as u64
}

// SYS_KILL — send a signal to a process

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

// SYS_WAIT — wait for a child process to exit

/// `SYS_WAIT(pid)` — block until child `pid` exits, then return its exit code.
///
/// If the child is already a Zombie, reaps immediately.
/// If `pid` is not a child of the caller, returns -ESRCH.
pub unsafe fn sys_wait(pid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(pid as u32)
}

// SYS_SLEEP — sleep for N milliseconds

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

// HelixFS Syscall Implementations

const ENOSYS: u64 = u64::MAX - 37;
const EINVAL: u64 = u64::MAX;
const EPERM: u64 = u64::MAX - 1;
const ENOENT: u64 = u64::MAX - 2;
const ESRCH: u64 = u64::MAX - 3;
const EIO: u64 = u64::MAX - 5;
const EBADF: u64 = u64::MAX - 9;
const ENOMEM: u64 = u64::MAX - 12;
const EFAULT: u64 = u64::MAX - 14;
const ENOTDIR: u64 = u64::MAX - 20;
const EPIPE: u64 = u64::MAX - 32;

// USER-POINTER VALIDATION

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
    if !validate_user_buf(ptr, len) {
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
    // Pipe-aware close: decrement refcounts on pipe ends.
    if let Ok(desc) = fd_table.get(fd as usize) {
        let pipe_idx = desc.mount_idx;
        if desc.flags & O_PIPE_READ != 0 {
            crate::pipe::pipe_close_reader(pipe_idx);
        }
        if desc.flags & O_PIPE_WRITE != 0 {
            crate::pipe::pipe_close_writer(pipe_idx);
        }
    }
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
                if !validate_user_buf(
                    stat_buf,
                    core::mem::size_of::<morpheus_helix::types::FileStat>() as u64,
                ) {
                    return EFAULT;
                }
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
                let entry_size = core::mem::size_of::<morpheus_helix::types::DirEntry>() as u64;
                let total_size = (count as u64).saturating_mul(entry_size);
                if !validate_user_buf(buf_ptr, total_size) {
                    return EFAULT;
                }
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

// SYS_CLOCK — monotonic nanoseconds since boot (TSC-based)

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

// SYS_SYSINFO — fill a SysInfo struct for the caller

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

// SYS_GETPPID — parent process ID

/// `SYS_GETPPID() → parent_pid`
pub unsafe fn sys_getppid() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    proc.parent_pid as u64
}

// SYS_SPAWN — spawn a child process from an ELF path in the VFS

/// `SYS_SPAWN(path_ptr, path_len, argv_ptr, argc) → child_pid`
///
/// Reads an ELF binary from the filesystem, loads it, and spawns a new
/// user process with optional argument passing and fd inheritance.
/// `argv_ptr` points to an array of `[ptr, len]` pairs (each 2×u64).
/// `argc` is the number of arguments (0 = no args).
pub unsafe fn sys_spawn(path_ptr: u64, path_len: u64, argv_ptr: u64, argc: u64) -> u64 {
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
    let pages_needed = file_size.div_ceil(4096) as u64;
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
    let bytes_read =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
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

    // Build null-separated argument blob from user argv array.
    let mut arg_blob = [0u8; 256];
    let mut blob_len: usize = 0;
    let mut arg_count: u8 = 0;
    if argc > 0 && argc <= 16 && argv_ptr != 0 {
        let argv_size = argc.saturating_mul(16); // each pair is [u64; 2] = 16 bytes
        if !validate_user_buf(argv_ptr, argv_size) {
            let _ = registry.free_pages(buf_phys, pages_needed);
            return EFAULT;
        }
        let argv = core::slice::from_raw_parts(argv_ptr as *const [u64; 2], argc as usize);
        for pair in argv.iter() {
            let a_ptr = pair[0];
            let a_len = pair[1] as usize;
            if a_ptr == 0 || a_len == 0 || a_len > 127 {
                continue;
            }
            if !validate_user_buf(a_ptr, a_len as u64) {
                continue;
            }
            if blob_len + a_len + 1 > 256 {
                break;
            }
            let src = core::slice::from_raw_parts(a_ptr as *const u8, a_len);
            arg_blob[blob_len..blob_len + a_len].copy_from_slice(src);
            blob_len += a_len;
            arg_blob[blob_len] = 0; // null separator
            blob_len += 1;
            arg_count += 1;
        }
    }

    // Spawn the process with fd inheritance and arguments.
    let elf_data = &buf[..bytes_read];
    let result = crate::process::scheduler::spawn_user_process(
        name,
        elf_data,
        &arg_blob[..blob_len],
        arg_count,
        true, // inherit fds from parent
    );

    // Free the temporary buffer.
    let _ = registry.free_pages(buf_phys, pages_needed);

    match result {
        Ok(pid) => pid as u64,
        Err(_) => ENOMEM,
    }
}

// SYS_MMAP — allocate + map pages into user virtual address space

/// Starting virtual address for user mmap allocations.
const USER_MMAP_BASE: u64 = 0x0000_0040_0000_0000;

/// `SYS_MMAP(pages) → virt_addr`
///
/// Allocates physical pages from MemoryRegistry, maps them into the
/// calling process's address space at the next available virtual address,
/// zeroes the memory, records the mapping in the process VMA table,
/// and returns that virtual address.
///
/// Returns `-EINVAL` for bad args, `-ENOMEM` on allocation failure,
/// `-ENOSYS` for PID 0 (kernel shares identity-mapped page table).
pub unsafe fn sys_mmap(pages: u64) -> u64 {
    if pages == 0 || pages > 4096 {
        return EINVAL;
    }
    if !crate::memory::is_registry_initialized() {
        return ENOMEM;
    }

    // PID 0 uses the kernel identity-mapped page table.
    // Mapping user pages into it would corrupt kernel mappings.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
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

    // Record the mapping in the VMA table.
    if proc.vma_table.insert(vaddr, phys, pages, true).is_err() {
        // VMA table full — unmap and free.
        let mut ptm2 = crate::paging::table::PageTableManager {
            pml4_phys: proc.cr3,
        };
        for i in 0..pages {
            let _ = ptm2.unmap_4k(vaddr + i * 4096);
        }
        let _ = registry.free_pages(phys, pages);
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    proc.pages_allocated += pages;

    // ── Full TLB + paging-structure cache flush ──────────────────────
    // map_user_page calls invlpg per-page, but COW at intermediate
    // levels (PML4→PDPT→PD) can leave stale paging-structure cache
    // entries for other addresses sharing those levels.  A CR3
    // write-back flushes everything, guaranteeing any thread sharing
    // this address space sees the new mappings.
    core::arch::asm!("mov {tmp}, cr3", "mov cr3, {tmp}", tmp = out(reg) _);

    vaddr
}
// SYS_MUNMAP — unmap pages from user virtual address space

/// `SYS_MUNMAP(vaddr, pages) → 0`
///
/// Unmaps pages from the calling process's address space.
/// If the region was allocated by SYS_MMAP (owns_phys == true), the
/// physical pages are freed back to the buddy allocator.
///
/// The `vaddr` must match the exact base address of a VMA entry and
/// `pages` must match its size.  Partial unmaps are not supported.
pub unsafe fn sys_munmap(vaddr: u64, pages: u64) -> u64 {
    if vaddr == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    // Ensure the address is page-aligned and in user space.
    if vaddr & 0xFFF != 0 || vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    // PID 0 never creates user VMAs.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
    }

    let proc = SCHEDULER.current_process_mut();

    // Find the VMA entry for this address.
    let (idx, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL, // not a known mapping
    };

    // Require exact size match (no partial munmap).
    if vma.pages != pages {
        return EINVAL;
    }

    let phys = vma.phys;
    let owns = vma.owns_phys;

    // Remove the VMA entry first (before any page table manipulation).
    proc.vma_table.remove(idx);

    // Unmap from the process's own page table.
    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: proc.cr3,
    };
    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let _ = ptm.unmap_4k(page_virt);
    }

    // If we own the physical pages, free them back to the allocator.
    if owns {
        let registry = crate::memory::global_registry_mut();
        let _ = registry.free_pages(phys, pages);
    }

    if proc.pages_allocated >= pages {
        proc.pages_allocated -= pages;
    }

    0
}

// SYS_DUP — duplicate a file descriptor

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

// SYS_SYSLOG — write to kernel serial log

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

// SYS_GETCWD — get current working directory

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

// SYS_CHDIR — change current working directory

/// `SYS_CHDIR(path_ptr, path_len) → 0`
///
/// Changes the calling process's working directory to the given path.
/// Returns `-ENOENT` if the path does not exist in the VFS.
pub unsafe fn sys_chdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    // Root always exists.
    if path == "/" {
        let proc = SCHEDULER.current_process_mut();
        proc.set_cwd(path);
        return 0;
    }

    // Verify path exists and is a directory via VFS stat.
    let fs = match morpheus_helix::vfs::global::fs_global() {
        Some(fs) => fs,
        None => return ENOSYS,
    };
    match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(stat) => {
            if !stat.is_dir {
                return ENOTDIR;
            }
            let proc = SCHEDULER.current_process_mut();
            proc.set_cwd(path);
            0
        }
        Err(_) => ENOENT,
    }
}

// PERSISTENCE — Key-Value store backed by HelixFS /persist/ directory
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
    let bytes =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
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
            path_buf[prefix.len()..prefix.len() + name_bytes.len()].copy_from_slice(name_bytes);
            if let Ok(p) = core::str::from_utf8(&path_buf[..prefix.len() + name_bytes.len()]) {
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

// SYS_PE_INFO — Binary introspection (PE + ELF)
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
    let pages_needed = read_size.div_ceil(4096) as u64;

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
    let bytes_read =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
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

    // detect elf
    if bytes_read >= 64 && data[0] == 0x7f && data[1] == b'E' && data[2] == b'L' && data[3] == b'F'
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
                data[24], data[25], data[26], data[27], data[28], data[29], data[30], data[31],
            ]);
            info.num_sections = u16::from_le_bytes([data[60], data[61]]) as u32;
        }
    }
    // detect pe/mz
    else if bytes_read >= 256 && data[0] == b'M' && data[1] == b'Z' {
        info.format = 2; // PE32+
        if let Ok(pe) =
            morpheus_persistent::pe::header::PeHeaders::parse(buf_phys as *const u8, bytes_read)
        {
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
    }

    let _ = registry.free_pages(buf_phys, pages_needed);

    core::ptr::write(info_ptr as *mut BinaryInfo, info);
    0
}

// NIC REGISTRATION — function-pointer bridge for network drivers
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
    /// Hardware control — set promisc, MAC, VLAN, offloads, etc.
    /// `cmd` selects the operation, `arg` is command-specific.
    /// Returns 0 on success, negative on error.
    pub ctrl: Option<unsafe fn(cmd: u32, arg: u64) -> i64>,
}

// nic_ctrl command constants
/// Enable/disable promiscuous mode.  arg: 1=on, 0=off.
pub const NIC_CTRL_PROMISC: u32 = 1;
/// Set MAC address (arg = pointer to 6 bytes).
pub const NIC_CTRL_MAC_SET: u32 = 2;
/// Get hardware statistics (arg = pointer to NicHwStats).
pub const NIC_CTRL_STATS: u32 = 3;
/// Reset hardware statistics counters.
pub const NIC_CTRL_STATS_RESET: u32 = 4;
/// Set MTU.  arg = new MTU value.
pub const NIC_CTRL_MTU: u32 = 5;
/// Enable/disable multicast (arg: 1=accept all, 0=filter).
pub const NIC_CTRL_MULTICAST: u32 = 6;
/// Set VLAN tag (arg: 0=disable, 1..4095=VLAN ID).
pub const NIC_CTRL_VLAN: u32 = 7;
/// Enable/disable TX checksum offload (arg: 1=on, 0=off).
pub const NIC_CTRL_TX_CSUM: u32 = 8;
/// Enable/disable RX checksum offload (arg: 1=on, 0=off).
pub const NIC_CTRL_RX_CSUM: u32 = 9;
/// Enable/disable TCP segmentation offload (arg: 1=on, 0=off).
pub const NIC_CTRL_TSO: u32 = 10;
/// Set RX ring buffer size (arg: number of descriptors).
pub const NIC_CTRL_RX_RING_SIZE: u32 = 11;
/// Set TX ring buffer size (arg: number of descriptors).
pub const NIC_CTRL_TX_RING_SIZE: u32 = 12;
/// Set interrupt coalescing (arg: microseconds between interrupts).
pub const NIC_CTRL_IRQ_COALESCE: u32 = 13;
/// Get NIC capabilities bitmask (arg = pointer to u64 out).
pub const NIC_CTRL_CAPS: u32 = 14;

/// Hardware NIC statistics (returned by NIC_CTRL_STATS).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NicHwStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub rx_dropped: u64,
    pub rx_crc_errors: u64,
    pub collisions: u64,
}

/// NIC capability bits (returned by NIC_CTRL_CAPS).
pub const NIC_CAP_PROMISC: u64 = 1 << 0;
pub const NIC_CAP_MAC_SET: u64 = 1 << 1;
pub const NIC_CAP_MULTICAST: u64 = 1 << 2;
pub const NIC_CAP_VLAN: u64 = 1 << 3;
pub const NIC_CAP_TX_CSUM: u64 = 1 << 4;
pub const NIC_CAP_RX_CSUM: u64 = 1 << 5;
pub const NIC_CAP_TSO: u64 = 1 << 6;
pub const NIC_CAP_IRQ_COALESCE: u64 = 1 << 7;

static mut NIC_OPS: NicOps = NicOps {
    tx: None,
    rx: None,
    link_up: None,
    mac: None,
    refill: None,
    ctrl: None,
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

// FRAMEBUFFER REGISTRATION — pass FB info from bootloader to hwinit

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

// SYS_NIC_INFO (32) — get NIC information

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

// SYS_NIC_TX (33) — transmit a raw Ethernet frame

/// `SYS_NIC_TX(frame_ptr, frame_len) → 0`
pub unsafe fn sys_nic_tx(frame_ptr: u64, frame_len: u64) -> u64 {
    if !validate_user_buf(frame_ptr, frame_len) {
        return EFAULT;
    }
    if !(14..=9000).contains(&frame_len) {
        return EINVAL; // min Ethernet header, max jumbo frame
    }
    match NIC_OPS.tx {
        Some(tx_fn) => {
            let rc = tx_fn(frame_ptr as *const u8, frame_len as usize);
            if rc < 0 {
                EIO
            } else {
                0
            }
        }
        None => ENODEV,
    }
}

// SYS_NIC_RX (34) — receive a raw Ethernet frame

/// `SYS_NIC_RX(buf_ptr, buf_len) → bytes_received`
pub unsafe fn sys_nic_rx(buf_ptr: u64, buf_len: u64) -> u64 {
    if !validate_user_buf(buf_ptr, buf_len) {
        return EFAULT;
    }
    match NIC_OPS.rx {
        Some(rx_fn) => {
            let rc = rx_fn(buf_ptr as *mut u8, buf_len as usize);
            if rc < 0 {
                EIO
            } else {
                rc as u64
            }
        }
        None => ENODEV,
    }
}

// SYS_NIC_LINK (35) — get link status

/// `SYS_NIC_LINK() → 0/1 (down/up)`
pub unsafe fn sys_nic_link() -> u64 {
    match NIC_OPS.link_up {
        Some(f) => f() as u64,
        None => ENODEV,
    }
}

// SYS_NIC_MAC (36) — get 6-byte MAC address

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

// SYS_NIC_REFILL (37) — refill RX descriptor ring

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

// NIC_CTRL — hardware-level NIC control (exokernel)

/// `sys_nic_ctrl(cmd, arg) → 0`
///
/// Direct hardware control: promiscuous mode, MAC spoofing, VLAN,
/// checksum offloads, ring sizing, interrupt coalescing, etc.
pub unsafe fn sys_nic_ctrl(cmd: u64, arg: u64) -> u64 {
    match NIC_OPS.ctrl {
        Some(f) => {
            let rc = f(cmd as u32, arg);
            if rc < 0 {
                EIO
            } else {
                rc as u64
            }
        }
        None => ENODEV,
    }
}

// SYS_IOCTL (42) — device control

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
        // Terminal window size: derive from framebuffer if available, else 80×25.
        (0..=2, IOCTL_TIOCGWINSZ) => {
            if arg != 0 && validate_user_buf(arg, 8) {
                let (rows, cols, xpix, ypix) = match FB_REGISTERED {
                    Some(fb) => {
                        let c = fb.width / 8; // 8px font width
                        let r = fb.height / 16; // 16px font height
                        (r as u16, c as u16, fb.width as u16, fb.height as u16)
                    }
                    None => (25, 80, 0, 0),
                };
                let buf = arg as *mut u16;
                *buf = rows; // ws_row
                *buf.add(1) = cols; // ws_col
                *buf.add(2) = xpix; // ws_xpixel
                *buf.add(3) = ypix; // ws_ypixel
            }
            0
        }
        _ => EINVAL,
    }
}

// SYS_MOUNT (43) — mount a filesystem

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

// SYS_UMOUNT (44) — unmount a filesystem

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

// SYS_POLL (45) — poll file descriptors for readiness

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

// SYS_PORT_IN (52) — read from I/O port

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

// SYS_PORT_OUT (53) — write to I/O port

/// `SYS_PORT_OUT(port, width, value) → 0`
///
/// Write to an x86 I/O port.  `width` is 1, 2, or 4.
pub unsafe fn sys_port_out(port: u64, width: u64, value: u64) -> u64 {
    if port > 0xFFFF {
        return EINVAL;
    }
    let port = port as u16;
    match width {
        1 => {
            crate::cpu::pio::outb(port, value as u8);
            0
        }
        2 => {
            crate::cpu::pio::outw(port, value as u16);
            0
        }
        4 => {
            crate::cpu::pio::outl(port, value as u32);
            0
        }
        _ => EINVAL,
    }
}

// SYS_PCI_CFG_READ (54) — read PCI configuration space

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
    let addr = crate::pci::PciAddr {
        bus,
        device: dev,
        function: func,
    };
    let off = offset as u8;
    match width {
        1 => crate::pci::pci_cfg_read8(addr, off) as u64,
        2 => crate::pci::pci_cfg_read16(addr, off) as u64,
        4 => crate::pci::pci_cfg_read32(addr, off) as u64,
        _ => EINVAL,
    }
}

// SYS_PCI_CFG_WRITE (55) — write PCI configuration space

/// `SYS_PCI_CFG_WRITE(bdf, offset, width, value) → 0`
pub unsafe fn sys_pci_cfg_write(bdf: u64, offset: u64, width: u64, value: u64) -> u64 {
    let bus = ((bdf >> 16) & 0xFF) as u8;
    let dev = ((bdf >> 8) & 0x1F) as u8;
    let func = (bdf & 0x07) as u8;
    if offset > 255 {
        return EINVAL;
    }
    let addr = crate::pci::PciAddr {
        bus,
        device: dev,
        function: func,
    };
    let off = offset as u8;
    match width {
        1 => {
            crate::pci::pci_cfg_write8(addr, off, value as u8);
            0
        }
        2 => {
            crate::pci::pci_cfg_write16(addr, off, value as u16);
            0
        }
        4 => {
            crate::pci::pci_cfg_write32(addr, off, value as u32);
            0
        }
        _ => EINVAL,
    }
}

// SYS_DMA_ALLOC (56) — allocate DMA-safe memory below 4GB

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

// SYS_DMA_FREE (57) — free DMA memory

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

// SYS_MAP_PHYS (58) — map physical address into process virtual space

/// `SYS_MAP_PHYS(phys, pages, flags) → virt_addr`
///
/// Maps `pages` 4K pages starting at physical address `phys` into the
/// calling process's virtual address space.  The physical memory is NOT
/// owned by the process — MUNMAP will unmap the PTEs but not free the
/// physical pages.
///
/// Flags: bit 0 = writable, bit 1 = uncacheable.
pub unsafe fn sys_map_phys(phys: u64, pages: u64, flags: u64) -> u64 {
    if phys == 0 || pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if phys & 0xFFF != 0 {
        return EINVAL; // must be page-aligned
    }

    // PID 0 uses the kernel identity-mapped page table.
    if SCHEDULER.current_pid() == 0 {
        return ENOSYS;
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

    // Record VMA (owns_phys = false: physical pages are not ours to free).
    if proc.vma_table.insert(vaddr, phys, pages, false).is_err() {
        // VMA table full — unmap what we just mapped.
        let mut ptm2 = crate::paging::table::PageTableManager {
            pml4_phys: proc.cr3,
        };
        for i in 0..pages {
            let _ = ptm2.unmap_4k(vaddr + i * 4096);
        }
        return ENOMEM;
    }

    proc.mmap_brk = vaddr + pages * 4096;
    vaddr
}

// SYS_VIRT_TO_PHYS (59) — translate virtual to physical address

/// `SYS_VIRT_TO_PHYS(virt) → phys`
///
/// Walk the calling process's page table to resolve a user virtual address
/// to its physical address.  Kernel addresses are rejected to prevent
/// information leaks.
pub unsafe fn sys_virt_to_phys(virt: u64) -> u64 {
    if virt >= USER_ADDR_LIMIT {
        return EFAULT;
    }
    match crate::paging::kvirt_to_phys(virt) {
        Some(phys) => phys,
        None => EINVAL,
    }
}

// SYS_IRQ_ATTACH (60) — enable an IRQ line

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

// SYS_IRQ_ACK (61) — acknowledge an IRQ (send EOI)

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

// SYS_CACHE_FLUSH (62) — flush CPU cache for an address range

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

// SYS_FB_INFO (63) — get framebuffer information

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

// SYS_FB_MAP (64) — map framebuffer into process virtual address space

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

    let pages = info.size.div_ceil(4096);
    // Use MAP_PHYS with writable + uncacheable flags.
    sys_map_phys(info.base, pages, 0x03) // flags: writable(1) | uncacheable(2)
}

// SYS_PS (65) — list all processes

/// Process info returned by SYS_PS.
/// Must match `libmorpheus::process::PsEntry` exactly.
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32, // 0=Ready, 1=Running, 2=Blocked, 3=Zombie, 4=Terminated
    pub priority: u32,
    pub cpu_ticks: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32], // NUL-terminated
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

// SYS_SIGACTION (66) — register a signal handler

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
    let sig_idx = signum as usize;
    let old = proc.signal_handlers[sig_idx];
    proc.signal_handlers[sig_idx] = handler;
    old
}

// SYS_SETPRIORITY (67) — set process scheduling priority

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

// SYS_GETPRIORITY (68) — get process scheduling priority

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

// SYS_CPUID (69) — execute CPUID instruction

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

// SYS_RDTSC (70) — read TSC with frequency info

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

// SYS_BOOT_LOG (71) — read kernel boot log

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

// SYS_MEMMAP (72) — read physical memory map

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

    let out = core::slice::from_raw_parts_mut(buf_ptr as *mut MemmapEntry, max_entries as usize);
    let count = total.min(max_entries as usize);

    for (i, slot) in out.iter_mut().enumerate().take(count) {
        if let Some(desc) = registry.get_descriptor(i) {
            *slot = MemmapEntry {
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

// NETWORK STACK — function-pointer bridge (TCP, DNS, config, poll)
//
// Like NicOps, the bootloader registers these after initialising the smoltcp
// stack.  hwinit has zero dependency on smoltcp — everything crosses the
// boundary as raw u64 / pointers / packed IPv4.
//
// Socket handles are opaque i64 values (smoltcp SocketHandle ordinals).
// Negative returns indicate errors.
//
// The raw NIC layer (32-37) + NIC_CTRL gives userspace full hardware
// control — userspace can build entirely custom protocol stacks from
// scratch.  The NET/DNS/CFG/POLL layer (38-41) is the *convenience*
// smoltcp-backed stack for programs that want TCP/IP without writing
// their own.  Both coexist; neither depends on the other.

// sub-commands for sys_net (38)
pub const NET_TCP_SOCKET: u64 = 0;
pub const NET_TCP_CONNECT: u64 = 1;
pub const NET_TCP_SEND: u64 = 2;
pub const NET_TCP_RECV: u64 = 3;
pub const NET_TCP_CLOSE: u64 = 4;
pub const NET_TCP_STATE: u64 = 5;
pub const NET_TCP_LISTEN: u64 = 6;
pub const NET_TCP_ACCEPT: u64 = 7;
pub const NET_TCP_SHUTDOWN: u64 = 8;
pub const NET_TCP_NODELAY: u64 = 9;
pub const NET_TCP_KEEPALIVE: u64 = 10;
// udp sub-commands for sys_net (38)
pub const NET_UDP_SOCKET: u64 = 11;
pub const NET_UDP_SEND_TO: u64 = 12;
pub const NET_UDP_RECV_FROM: u64 = 13;
pub const NET_UDP_CLOSE: u64 = 14;

// sub-commands for sys_dns (39)
pub const DNS_START: u64 = 0;
pub const DNS_RESULT: u64 = 1;
pub const DNS_SET_SERVERS: u64 = 2;

// sub-commands for sys_net_cfg (40)
pub const NET_CFG_GET: u64 = 0;
pub const NET_CFG_DHCP: u64 = 1;
pub const NET_CFG_STATIC: u64 = 2;
pub const NET_CFG_HOSTNAME: u64 = 3;

// sub-commands for sys_net_poll (41)
pub const NET_POLL_DRIVE: u64 = 0;
pub const NET_POLL_STATS: u64 = 1;

/// Network stack configuration snapshot, returned by NET_CFG_GET.
///
/// Packed C layout — userspace casts the result buffer to this.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetConfigInfo {
    /// Stack state: 0=unconfigured, 1=dhcp_discovering, 2=ready, 3=error.
    pub state: u32,
    /// Bit 0: DHCP active, bit 1: has gateway, bit 2: has DNS.
    pub flags: u32,
    /// IPv4 address (network byte order).
    pub ipv4_addr: u32,
    /// CIDR prefix length.
    pub prefix_len: u8,
    pub _pad0: [u8; 3],
    /// Gateway IPv4 (network byte order).
    pub gateway: u32,
    /// Primary DNS (network byte order).
    pub dns_primary: u32,
    /// Secondary DNS (network byte order).
    pub dns_secondary: u32,
    /// Current MAC address (6 bytes).
    pub mac: [u8; 6],
    pub _pad1: [u8; 2],
    /// Current MTU.
    pub mtu: u32,
    /// NUL-terminated hostname (max 63 + NUL).
    pub hostname: [u8; 64],
}

/// Network statistics, returned by NET_POLL_STATS.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    /// Number of active TCP sockets.
    pub tcp_active: u32,
    pub _pad: u32,
}

/// UDP send function signature.
type UdpSendFn =
    unsafe fn(handle: i64, dest_ip: u32, dest_port: u16, buf: *const u8, len: usize) -> i64;
/// UDP receive function signature.
type UdpRecvFn = unsafe fn(handle: i64, buf: *mut u8, len: usize, src_out: *mut u8) -> i64;

/// Network stack function-pointer table.
///
/// Registered by the bootloader after it creates a `NetInterface<D>`.
/// Every function returns >=0 on success (or a meaningful value),
/// negative on error.
#[repr(C)]
pub struct NetStackOps {
    // tcp
    /// Create a TCP socket.  Returns handle (>=0) or negative error.
    pub tcp_socket: Option<unsafe fn() -> i64>,
    /// Connect.  `ip` is network-byte-order IPv4.
    pub tcp_connect: Option<unsafe fn(handle: i64, ip: u32, port: u16) -> i64>,
    /// Send.  Returns bytes sent (>=0) or negative error.
    pub tcp_send: Option<unsafe fn(handle: i64, buf: *const u8, len: usize) -> i64>,
    /// Receive.  Returns bytes received (>=0) or negative error.
    pub tcp_recv: Option<unsafe fn(handle: i64, buf: *mut u8, len: usize) -> i64>,
    /// Close a socket.
    pub tcp_close: Option<unsafe fn(handle: i64)>,
    /// Query TCP state.  Returns state ordinal (0=Closed..10=TimeWait).
    pub tcp_state: Option<unsafe fn(handle: i64) -> i64>,
    /// Bind + listen on a local port.  Returns 0 or negative error.
    pub tcp_listen: Option<unsafe fn(handle: i64, port: u16) -> i64>,
    /// Accept an incoming connection.  Returns new handle or negative.
    pub tcp_accept: Option<unsafe fn(listen_handle: i64) -> i64>,
    /// Half-close: shutdown write side.  Returns 0 or negative.
    pub tcp_shutdown: Option<unsafe fn(handle: i64) -> i64>,
    /// Set TCP_NODELAY (Nagle disable). arg: 1=on 0=off.
    pub tcp_nodelay: Option<unsafe fn(handle: i64, on: i64) -> i64>,
    /// Set keepalive interval in ms.  0=disable.
    pub tcp_keepalive: Option<unsafe fn(handle: i64, ms: u64) -> i64>,

    // udp
    /// Create a UDP socket.  Returns handle (>=0) or negative error.
    pub udp_socket: Option<unsafe fn() -> i64>,
    /// Send a datagram.  `dest_ip` is NBO IPv4.  Returns bytes sent.
    pub udp_send_to: Option<UdpSendFn>,
    /// Receive a datagram.  Writes sender IP (NBO) + port into `src_out`.
    /// Returns bytes received (>=0), 0 if nothing available.
    /// `src_out` layout: [u32 ip_nbo, u16 port, u16 _pad] = 8 bytes.
    pub udp_recv_from: Option<UdpRecvFn>,
    /// Close a UDP socket.
    pub udp_close: Option<unsafe fn(handle: i64)>,

    // dns
    /// Start an async DNS query.  Returns query handle or negative.
    pub dns_start: Option<unsafe fn(name: *const u8, len: usize) -> i64>,
    /// Poll a DNS query.  Writes 4-byte IPv4 to `out`.
    /// Returns 0 if resolved, 1 if pending, negative on error.
    pub dns_result: Option<unsafe fn(query: i64, out: *mut u8) -> i64>,
    /// Override DNS servers.  `servers` points to packed u32 IPv4 addrs.
    pub dns_set_servers: Option<unsafe fn(servers: *const u32, count: usize) -> i64>,

    // configuration
    /// Fill a `NetConfigInfo` at `buf`.
    pub cfg_get: Option<unsafe fn(buf: *mut u8) -> i64>,
    /// Switch to DHCP mode.
    pub cfg_dhcp: Option<unsafe fn() -> i64>,
    /// Set a static IPv4.  `ip` and `gateway` are NBO.
    pub cfg_static_ip: Option<unsafe fn(ip: u32, prefix_len: u8, gateway: u32) -> i64>,
    /// Set the hostname (for DHCP FQDN option, etc.).
    pub cfg_hostname: Option<unsafe fn(name: *const u8, len: usize) -> i64>,

    // poll / stats
    /// Drive the smoltcp stack (DHCP, ARP, TCP timers).  Returns 1 if
    /// any socket activity occurred, 0 otherwise.
    pub poll_drive: Option<unsafe fn(timestamp_ms: u64) -> i64>,
    /// Fill a `NetStats` at `buf`.
    pub poll_stats: Option<unsafe fn(buf: *mut u8) -> i64>,
}

static mut NET_STACK_OPS: NetStackOps = NetStackOps {
    tcp_socket: None,
    tcp_connect: None,
    tcp_send: None,
    tcp_recv: None,
    tcp_close: None,
    tcp_state: None,
    tcp_listen: None,
    tcp_accept: None,
    tcp_shutdown: None,
    tcp_nodelay: None,
    tcp_keepalive: None,
    udp_socket: None,
    udp_send_to: None,
    udp_recv_from: None,
    udp_close: None,
    dns_start: None,
    dns_result: None,
    dns_set_servers: None,
    cfg_get: None,
    cfg_dhcp: None,
    cfg_static_ip: None,
    cfg_hostname: None,
    poll_drive: None,
    poll_stats: None,
};

/// Register network stack function pointers.
///
/// Called by the bootloader after it creates a `NetInterface<D>` and
/// wraps its methods into `unsafe fn` closures.
pub unsafe fn register_net_stack(ops: NetStackOps) {
    NET_STACK_OPS = ops;
}

/// Check if a network stack is registered.
fn net_stack_present() -> bool {
    unsafe { NET_STACK_OPS.tcp_socket.is_some() }
}

const ENOSYS_NET: u64 = u64::MAX - 37;

// SYS_NET (38) — TCP socket operations (multiplexed via subcmd)

/// `SYS_NET(subcmd, a2, a3, a4) → result`
pub unsafe fn sys_net(subcmd: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // TCP_SOCKET() → handle
        NET_TCP_SOCKET => match NET_STACK_OPS.tcp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            }
            None => ENOSYS_NET,
        },
        // TCP_CONNECT(handle, ipv4_nbo, port) → 0
        NET_TCP_CONNECT => {
            let handle = a2 as i64;
            let ip = a3 as u32;
            let port = a4 as u16;
            match NET_STACK_OPS.tcp_connect {
                Some(f) => {
                    let rc = f(handle, ip, port);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_SEND(handle, buf_ptr, buf_len) → bytes_sent
        NET_TCP_SEND => {
            let handle = a2 as i64;
            if a4 > 0 && !validate_user_buf(a3, a4) {
                return EFAULT;
            }
            match NET_STACK_OPS.tcp_send {
                Some(f) => {
                    let rc = f(handle, a3 as *const u8, a4 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_RECV(handle, buf_ptr, buf_len) → bytes_received
        NET_TCP_RECV => {
            let handle = a2 as i64;
            if a4 > 0 && !validate_user_buf(a3, a4) {
                return EFAULT;
            }
            match NET_STACK_OPS.tcp_recv {
                Some(f) => {
                    let rc = f(handle, a3 as *mut u8, a4 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_CLOSE(handle) → 0
        NET_TCP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_close {
                Some(f) => {
                    f(handle);
                    0
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_STATE(handle) → state ordinal
        NET_TCP_STATE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_state {
                Some(f) => {
                    let s = f(handle);
                    if s < 0 {
                        EINVAL
                    } else {
                        s as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_LISTEN(handle, port) → 0
        NET_TCP_LISTEN => {
            let handle = a2 as i64;
            let port = a3 as u16;
            match NET_STACK_OPS.tcp_listen {
                Some(f) => {
                    let rc = f(handle, port);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_ACCEPT(listen_handle) → new handle
        NET_TCP_ACCEPT => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_accept {
                Some(f) => {
                    let h = f(handle);
                    if h < 0 {
                        EIO
                    } else {
                        h as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_SHUTDOWN(handle) → 0
        NET_TCP_SHUTDOWN => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_shutdown {
                Some(f) => {
                    let rc = f(handle);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_NODELAY(handle, on) → 0
        NET_TCP_NODELAY => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_nodelay {
                Some(f) => {
                    let rc = f(handle, a3 as i64);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // TCP_KEEPALIVE(handle, interval_ms) → 0
        NET_TCP_KEEPALIVE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.tcp_keepalive {
                Some(f) => {
                    let rc = f(handle, a3);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // udp sub-commands
        // UDP_SOCKET() → handle
        NET_UDP_SOCKET => match NET_STACK_OPS.udp_socket {
            Some(f) => {
                let h = f();
                if h < 0 {
                    ENOMEM
                } else {
                    h as u64
                }
            }
            None => ENOSYS_NET,
        },
        // UDP_SEND_TO(handle, dest_ip_nbo, dest_port | buf_ptr, buf_len)
        // a2 = handle, a3 = dest_ip_nbo | (port << 32), a4 = buf_ptr | (len << 32)
        // Re-pack: dest_ip in lower 32 of a3, port in upper 16 bits
        // Actually, with 4 args available (subcmd, a2, a3, a4):
        //   subcmd=12, a2=handle, a3=packed(ip:32|port:16|pad:16), a4=buf_ptr
        // But we need 5 args (handle, ip, port, buf, len). Solution: pack
        // ip+port into a3 and buf+len into a4 won't work (64-bit ptrs).
        // Use 5th arg via a4 as buf_ptr, and pass len via a2 upper bits.
        // Better: repack. handle=a2, dest_addr_ptr=a3 (8-byte struct), buf=a4.
        //
        // Cleanest ABI for UDP send_to with 4 args:
        //   a2 = handle
        //   a3 = pointer to UdpTarget { ip_nbo: u32, port: u16, _pad: u16 }
        //   a4 = pointer to (buf_ptr: u64, buf_len: u64) pair
        //
        // No — too many indirections. Use the 5-arg dispatch variant:
        //   The dispatch passes a1..a4 (4 user args after subcmd).
        //   a2=handle, a3=dest_ip_nbo, a4=dest_port|(len<<16), but len>65535
        //   is possible. Bad.
        //
        // Final design — message struct:
        //   a2 = handle
        //   a3 = pointer to UdpSendDesc { ip: u32, port: u16, _pad: u16, buf: *const u8, len: u64 }
        NET_UDP_SEND_TO => {
            let handle = a2 as i64;
            // a3 points to UdpSendDesc in user memory
            let desc_size = 24u64; // u32 + u16 + u16 + u64 + u64 = 24
            if !validate_user_buf(a3, desc_size) {
                return EFAULT;
            }
            let desc = a3 as *const u8;
            let ip = *(desc as *const u32);
            let port = *((desc.add(4)) as *const u16);
            let buf_ptr = *((desc.add(8)) as *const u64);
            let buf_len = *((desc.add(16)) as *const u64);
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            if buf_len > 65535 {
                return EINVAL;
            } // UDP max payload
            match NET_STACK_OPS.udp_send_to {
                Some(f) => {
                    let rc = f(handle, ip, port, buf_ptr as *const u8, buf_len as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // UDP_RECV_FROM(handle, buf_ptr, buf_len)
        //   a2 = handle
        //   a3 = pointer to UdpRecvDesc { buf: *mut u8, buf_len: u64, src_ip: u32, src_port: u16, _pad: u16 }
        NET_UDP_RECV_FROM => {
            let handle = a2 as i64;
            let desc_size = 24u64; // *mut u8(8) + u64(8) + u32(4) + u16(2) + u16(2) = 24
            if !validate_user_buf(a3, desc_size) {
                return EFAULT;
            }
            let desc = a3 as *mut u8;
            let buf_ptr = *(desc as *const u64);
            let buf_len = *((desc.add(8)) as *const u64);
            if buf_len > 0 && !validate_user_buf(buf_ptr, buf_len) {
                return EFAULT;
            }
            // src_out is at offset 16 in the desc (4 + 2 + 2 = 8 bytes for src info)
            let src_out = desc.add(16);
            match NET_STACK_OPS.udp_recv_from {
                Some(f) => {
                    let rc = f(handle, buf_ptr as *mut u8, buf_len as usize, src_out);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // UDP_CLOSE(handle) → 0
        NET_UDP_CLOSE => {
            let handle = a2 as i64;
            match NET_STACK_OPS.udp_close {
                Some(f) => {
                    f(handle);
                    0
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}

// SYS_DNS (39) — DNS resolution

/// `SYS_DNS(subcmd, a2, a3) → result`
pub unsafe fn sys_dns(subcmd: u64, a2: u64, a3: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // DNS_START(name_ptr, name_len) → query handle
        DNS_START => {
            if a3 == 0 || a3 > 253 {
                return EINVAL;
            }
            if !validate_user_buf(a2, a3) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_start {
                Some(f) => {
                    let h = f(a2 as *const u8, a3 as usize);
                    if h < 0 {
                        EIO
                    } else {
                        h as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // DNS_RESULT(query_handle, result_buf_ptr) → 0=resolved, 1=pending
        DNS_RESULT => {
            let query = a2 as i64;
            if !validate_user_buf(a3, 4) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_result {
                Some(f) => {
                    let rc = f(query, a3 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        rc as u64
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // DNS_SET_SERVERS(servers_ptr, count)
        DNS_SET_SERVERS => {
            let count = a3;
            if count == 0 || count > 4 {
                return EINVAL;
            }
            if !validate_user_buf(a2, count * 4) {
                return EFAULT;
            }
            match NET_STACK_OPS.dns_set_servers {
                Some(f) => {
                    let rc = f(a2 as *const u32, count as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}

// SYS_NET_CFG (40) — IP stack configuration

/// `SYS_NET_CFG(subcmd, a2, a3, a4) → result`
pub unsafe fn sys_net_cfg(subcmd: u64, a2: u64, a3: u64, _a4: u64) -> u64 {
    match subcmd {
        // CFG_GET(buf_ptr) — works even without stack (returns zeroed)
        NET_CFG_GET => {
            let size = core::mem::size_of::<NetConfigInfo>() as u64;
            if !validate_user_buf(a2, size) {
                return EFAULT;
            }
            match NET_STACK_OPS.cfg_get {
                Some(f) => {
                    let rc = f(a2 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => {
                    // No stack: zero-fill so userspace sees state=0 (unconfigured)
                    core::ptr::write_bytes(a2 as *mut u8, 0, size as usize);
                    0
                }
            }
        }
        // All remaining subcmds require the stack.
        _ if !net_stack_present() => ENODEV,

        // CFG_DHCP() — enable DHCP
        NET_CFG_DHCP => match NET_STACK_OPS.cfg_dhcp {
            Some(f) => {
                let rc = f();
                if rc < 0 {
                    EIO
                } else {
                    0
                }
            }
            None => ENOSYS_NET,
        },
        // CFG_STATIC(ip_nbo, prefix_gw_packed, 0)
        // prefix_gw_packed = (prefix_len << 32) | gateway_nbo
        NET_CFG_STATIC => {
            let ip_nbo = a2 as u32;
            let prefix_len = (a3 >> 32) as u8;
            let gw_nbo = a3 as u32;
            match NET_STACK_OPS.cfg_static_ip {
                Some(f) => {
                    let rc = f(ip_nbo, prefix_len, gw_nbo);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // CFG_HOSTNAME(name_ptr, name_len)
        NET_CFG_HOSTNAME => {
            if a3 == 0 || a3 > 63 {
                return EINVAL;
            }
            if !validate_user_buf(a2, a3) {
                return EFAULT;
            }
            match NET_STACK_OPS.cfg_hostname {
                Some(f) => {
                    let rc = f(a2 as *const u8, a3 as usize);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        // nic hardware control (subcmd >= 128)
        // These go directly to NicOps.ctrl, bypassing the IP stack.
        // This is the exokernel escape hatch: promisc, MAC spoof,
        // VLAN, offloads, ring sizing, interrupt coalescing.
        128.. => {
            let nic_cmd = (subcmd - 128) as u32;
            sys_nic_ctrl(nic_cmd as u64, a2)
        }
        _ => EINVAL,
    }
}

// SYS_NET_POLL (41) — drive the stack & query statistics

/// `SYS_NET_POLL(subcmd, a2) → result`
pub unsafe fn sys_net_poll(subcmd: u64, a2: u64) -> u64 {
    if !net_stack_present() {
        return ENODEV;
    }

    match subcmd {
        // POLL_DRIVE(timestamp_ms) → 0/1 (activity)
        NET_POLL_DRIVE => match NET_STACK_OPS.poll_drive {
            Some(f) => {
                let rc = f(a2);
                if rc < 0 {
                    EIO
                } else {
                    rc as u64
                }
            }
            None => ENOSYS_NET,
        },
        // POLL_STATS(buf_ptr) → 0
        NET_POLL_STATS => {
            let size = core::mem::size_of::<NetStats>() as u64;
            if !validate_user_buf(a2, size) {
                return EFAULT;
            }
            match NET_STACK_OPS.poll_stats {
                Some(f) => {
                    let rc = f(a2 as *mut u8);
                    if rc < 0 {
                        EIO
                    } else {
                        0
                    }
                }
                None => ENOSYS_NET,
            }
        }
        _ => EINVAL,
    }
}
// SYS_SHM_GRANT (73) — grant shared physical pages to another process
//
// Exokernel shared memory primitive.  The caller specifies physical pages
// it owns (via SYS_MMAP or SYS_DMA_ALLOC), and the kernel maps those same
// physical frames into the target process's address space.
//
// This is *unidirectional grant*, not symmetric attach.  The granting
// process retains its own mapping.  The target receives a new VMA with
// `owns_phys = false` so that munmap in the target does NOT free the
// physical pages (the granter still owns them).
//
// # Arguments
//
//   a1 = target_pid (u32)
//   a2 = source virtual address (must be start of a VMA in the caller)
//   a3 = number of 4 KiB pages (must match the VMA exactly)
//   a4 = flags: bit 0 = writable, bit 1 = executable
//
// # Returns
//
//   Virtual address in the target process, or error code.
//
// # Security model
//
//   - Only processes that OWN physical pages can grant them (owns_phys=true)
//   - The target process cannot free the underlying physical memory
//   - The granter can munmap its side, but the target's mapping persists
//     until the target munmaps or exits
//   - There is no ambient authority: you must know the PID and possess
//     a valid VMA

/// Protection flags for SYS_SHM_GRANT and SYS_MPROTECT.
pub const PROT_READ: u64 = 0; // Read-only (no additional bits)
pub const PROT_WRITE: u64 = 1; // Writable
pub const PROT_EXEC: u64 = 2; // Executable (clears NX)

/// `SYS_SHM_GRANT(target_pid, src_vaddr, pages, flags) → target_vaddr`
pub unsafe fn sys_shm_grant(target_pid: u64, src_vaddr: u64, pages: u64, flags: u64) -> u64 {
    // argument validation
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if src_vaddr == 0 || src_vaddr & 0xFFF != 0 {
        return EINVAL;
    }
    if src_vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }

    let caller_pid = SCHEDULER.current_pid();

    // Cannot grant to self (use mmap), cannot grant to kernel.
    if target_pid == 0 || target_pid == caller_pid as u64 {
        return EINVAL;
    }
    // Caller must not be the kernel.
    if caller_pid == 0 {
        return EPERM;
    }

    // verify source vma in the caller
    let caller_proc = SCHEDULER.current_process_mut();
    let (_, src_vma) = match caller_proc.vma_table.find_exact(src_vaddr) {
        Some(pair) => pair,
        None => return EINVAL, // not a known mapping
    };

    // Must match exact page count.
    if src_vma.pages != pages {
        return EINVAL;
    }

    // Only owned physical pages can be granted.  We refuse to re-grant
    // pages that were themselves granted (owns_phys=false), because the
    // original owner controls their lifetime.
    if !src_vma.owns_phys {
        return EPERM;
    }

    let phys = src_vma.phys;

    // validate target process
    let target_pid_u32 = target_pid as u32;
    let target_proc = match SCHEDULER.process_by_pid(target_pid_u32) {
        Some(p) => p,
        None => return ESRCH, // no such process
    };

    // Target must be alive (Ready, Running, or Blocked).
    if target_proc.is_free() {
        return ESRCH;
    }
    if target_proc.cr3 == 0 {
        return ESRCH; // kernel thread without user page table
    }

    // compute target virtual address
    // We need mutable access to the target.  Re-acquire via raw table
    // access since we can't hold two &mut through SCHEDULER.
    // SAFETY: single-core, interrupts disabled during syscall.
    let target = {
        use crate::process::scheduler::PROCESS_TABLE;
        match PROCESS_TABLE.get_mut(target_pid_u32 as usize) {
            Some(Some(p)) => p as *mut crate::process::Process,
            _ => return ESRCH,
        }
    };

    let target_ref = &mut *target;

    if target_ref.mmap_brk == 0 {
        target_ref.mmap_brk = USER_MMAP_BASE;
    }
    let target_vaddr = target_ref.mmap_brk;

    // build pte flags
    let mut pte_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    if flags & PROT_WRITE != 0 {
        pte_flags = pte_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if flags & PROT_EXEC != 0 {
        pte_flags = pte_flags.without(crate::paging::entry::PageFlags::NO_EXECUTE);
    }

    // map physical pages into target's address space
    let mut ptm = crate::paging::table::PageTableManager {
        pml4_phys: target_ref.cr3,
    };

    for i in 0..pages {
        let page_virt = target_vaddr + i * 4096;
        let page_phys = phys + i * 4096;
        if crate::elf::map_user_page(&mut ptm, page_virt, page_phys, pte_flags).is_err() {
            // Roll back: unmap pages we already mapped.
            let mut ptm2 = crate::paging::table::PageTableManager {
                pml4_phys: target_ref.cr3,
            };
            for j in 0..i {
                let _ = ptm2.unmap_4k(target_vaddr + j * 4096);
            }
            return ENOMEM;
        }
    }

    // record vma in target (owns_phys = false)
    if target_ref
        .vma_table
        .insert(target_vaddr, phys, pages, false)
        .is_err()
    {
        // VMA table full — unmap everything.
        let mut ptm3 = crate::paging::table::PageTableManager {
            pml4_phys: target_ref.cr3,
        };
        for i in 0..pages {
            let _ = ptm3.unmap_4k(target_vaddr + i * 4096);
        }
        return ENOMEM;
    }

    target_ref.mmap_brk = target_vaddr + pages * 4096;

    target_vaddr
}

// SYS_MPROTECT (74) — change page protection flags
//
// Modifies the x86-64 page table flags on an existing VMA in the calling
// process.  This is a bare page-table-flag-flip — the minimum kernel
// mechanism for W^X enforcement, guard pages, and JIT compilation.
//
// # Arguments
//
//   a1 = virtual address (must be the exact start of a VMA)
//   a2 = number of 4 KiB pages (must match the VMA exactly)
//   a3 = protection flags:
//        bit 0 (PROT_WRITE) = set WRITABLE
//        bit 1 (PROT_EXEC)  = clear NO_EXECUTE (allow execution)
//        All other bits must be zero.
//        PROT_READ is implied (a present page is always readable on x86-64).
//
// # Returns
//
//   0 on success, or error code.
//
// # Constraints
//
//   - Must match an existing VMA exactly (vaddr and pages).
//   - PROT_WRITE | PROT_EXEC simultaneously is allowed but discouraged
//     (breaks W^X).
//   - Does NOT split VMAs.  If you need different protections on sub-ranges,
//     mmap separate regions.

/// `SYS_MPROTECT(vaddr, pages, prot) → 0`
pub unsafe fn sys_mprotect(vaddr: u64, pages: u64, prot: u64) -> u64 {
    // argument validation
    if pages == 0 || pages > 1024 {
        return EINVAL;
    }
    if vaddr == 0 || vaddr & 0xFFF != 0 {
        return EINVAL;
    }
    if vaddr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    // Only bits 0 and 1 are defined.
    if prot & !3 != 0 {
        return EINVAL;
    }

    if SCHEDULER.current_pid() == 0 {
        return EPERM;
    }

    let proc = SCHEDULER.current_process_mut();

    // find the vma
    let (_, vma) = match proc.vma_table.find_exact(vaddr) {
        Some(pair) => pair,
        None => return EINVAL,
    };

    if vma.pages != pages {
        return EINVAL;
    }

    // build new pte flags
    // Base: PRESENT + USER + NX (read-only, non-executable)
    let mut new_flags = crate::paging::entry::PageFlags::PRESENT
        .with(crate::paging::entry::PageFlags::USER)
        .with(crate::paging::entry::PageFlags::NO_EXECUTE);

    if prot & PROT_WRITE != 0 {
        new_flags = new_flags.with(crate::paging::entry::PageFlags::WRITABLE);
    }
    if prot & PROT_EXEC != 0 {
        new_flags = new_flags.without(crate::paging::entry::PageFlags::NO_EXECUTE);
    }

    // walk and update each pte
    //
    // We walk the process's own page table tree.  Each 4 KiB page maps
    // to a leaf PTE at the PT level.  We rewrite the PTE preserving the
    // physical address but replacing the flag bits.
    //
    // This is safe because:
    //   1. We verified the VMA exists (so the pages ARE mapped).
    //   2. We only touch leaf PTEs in the user's page table.
    //   3. We flush the TLB after every PTE write.

    let pml4 = proc.cr3 as *mut crate::paging::entry::PageTable;

    for i in 0..pages {
        let page_virt = vaddr + i * 4096;
        let va = crate::paging::table::VirtAddr::from_u64(page_virt);

        // Walk PML4 → PDPT → PD → PT
        let pml4_e = (*pml4).entry(va.pml4_idx);
        if !pml4_e.is_present() {
            return EFAULT; // page table corruption — shouldn't happen
        }

        let pdpt = pml4_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pdpt_e = (*pdpt).entry(va.pdpt_idx);
        if !pdpt_e.is_present() {
            return EFAULT;
        }

        let pd = pdpt_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pd_e = (*pd).entry(va.pd_idx);
        if !pd_e.is_present() {
            return EFAULT;
        }
        if pd_e.is_huge() {
            // 2 MiB huge page — cannot mprotect sub-ranges of a huge page.
            // This shouldn't occur for user VMAs (we only map 4 KiB pages).
            return EINVAL;
        }

        let pt = pd_e.phys_addr() as *mut crate::paging::entry::PageTable;
        let pte = (*pt).entry_mut(va.pt_idx);

        if !pte.is_present() {
            return EFAULT; // VMA says it's mapped but PTE disagrees
        }

        // Preserve the physical address, replace flags.
        let phys_addr = pte.phys_addr();
        *pte = crate::paging::entry::PageTableEntry::new(phys_addr, new_flags);

        crate::paging::table::PageTableManager::flush_tlb_page(page_virt);
    }

    0
}

// SYS_PIPE (75) — create a unidirectional pipe

/// `SYS_PIPE(result_ptr) → 0`
///
/// Creates a pipe.  Writes `[read_fd, write_fd]` (two u64s) at `result_ptr`.
pub unsafe fn sys_pipe(result_ptr: u64) -> u64 {
    if !validate_user_buf(result_ptr, 8) {
        return EFAULT;
    }
    let pipe_idx = match crate::pipe::pipe_alloc() {
        Some(idx) => idx,
        None => return ENOMEM,
    };

    let fd_table = SCHEDULER.current_fd_table_mut();

    // Allocate read-end fd.
    let read_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => return ENOMEM,
    };
    fd_table.fds[read_fd] = morpheus_helix::types::FileDescriptor {
        key: 0,
        path: [0u8; 256],
        flags: O_PIPE_READ,
        offset: 0,
        mount_idx: pipe_idx,
        _pad: [0; 3],
        pinned_lsn: 0,
    };

    // Allocate write-end fd.
    let write_fd = match fd_table.alloc() {
        Ok(fd) => fd,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, read_fd);
            return ENOMEM;
        }
    };
    fd_table.fds[write_fd] = morpheus_helix::types::FileDescriptor {
        key: 0,
        path: [0u8; 256],
        flags: O_PIPE_WRITE,
        offset: 0,
        mount_idx: pipe_idx,
        _pad: [0; 3],
        pinned_lsn: 0,
    };

    // Write back as [u32; 2] — matches userspace `fds: [u32; 2]`.
    let out = result_ptr as *mut [u32; 2];
    (*out)[0] = read_fd as u32;
    (*out)[1] = write_fd as u32;
    0
}

// SYS_DUP2 (76) — duplicate a file descriptor

/// `SYS_DUP2(old_fd, new_fd) → new_fd`
///
/// Duplicate `old_fd` into `new_fd`.  If `new_fd` is already open it is
/// silently closed first.
pub unsafe fn sys_dup2(old_fd: u64, new_fd: u64) -> u64 {
    let fd_table = SCHEDULER.current_fd_table_mut();
    let src = match fd_table.get(old_fd as usize) {
        Ok(d) => *d,
        Err(_) => return EBADF,
    };

    // Close new_fd if it's in use (pipe-aware).
    if fd_table.get(new_fd as usize).is_ok() {
        sys_fs_close(new_fd);
    }

    // Ensure new_fd slot is within bounds.
    let fd_table = SCHEDULER.current_fd_table_mut();
    if new_fd as usize >= morpheus_helix::types::MAX_FDS {
        return EBADF;
    }

    // Place the duplicated descriptor.
    fd_table.fds[new_fd as usize] = src;

    // Bump pipe refcounts.
    if src.flags & O_PIPE_READ != 0 {
        crate::pipe::pipe_add_reader(src.mount_idx);
    }
    if src.flags & O_PIPE_WRITE != 0 {
        crate::pipe::pipe_add_writer(src.mount_idx);
    }

    new_fd
}

// SYS_SET_FG (77) — set foreground process for stdin

/// `SYS_SET_FG(pid) → 0`
pub unsafe fn sys_set_fg(pid: u64) -> u64 {
    crate::stdin::set_foreground_pid(pid as u32);
    0
}

// SYS_GETARGS (78) — retrieve command-line arguments

/// `SYS_GETARGS(buf_ptr, buf_len) → argc`
///
/// Copies the null-separated argument blob into the user buffer.
/// Returns the argument count (argc) in RAX.
pub unsafe fn sys_getargs(buf_ptr: u64, buf_len: u64) -> u64 {
    let proc = SCHEDULER.current_process_mut();
    let argc = proc.argc;
    let args_len = proc.args_len as usize;

    if buf_ptr != 0 && buf_len > 0 {
        let copy_len = core::cmp::min(args_len, buf_len as usize);
        if validate_user_buf(buf_ptr, copy_len as u64) {
            let dst = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, copy_len);
            dst.copy_from_slice(&proc.args[..copy_len]);
        }
    }

    argc as u64
}

// Helper — blocking pipe read

/// Read from a pipe, blocking if empty until data arrives or all writers close.
unsafe fn sys_pipe_read_blocking(pipe_idx: u8, buf: &mut [u8]) -> u64 {
    loop {
        let n = crate::pipe::pipe_read(pipe_idx, buf);
        if n > 0 {
            return n as u64;
        }
        // No data — if no writers remain, return EOF (0).
        if crate::pipe::pipe_writers(pipe_idx) == 0 {
            return 0;
        }
        // Block until a writer wakes us.
        {
            let proc = SCHEDULER.current_process_mut();
            proc.state = crate::process::ProcessState::Blocked(
                crate::process::BlockReason::PipeRead(pipe_idx),
            );
        }
        core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    }
}

// SYS_FUTEX (79) — userspace synchronization primitive

const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;

/// `SYS_FUTEX(addr, op, val, timeout_ms)` — futex wait/wake.
///
/// op=0 (WAIT): if `*addr == val`, block until woken or timeout_ms expires.
///              if `*addr != val`, return EAGAIN immediately.
///              timeout_ms=0 means wait forever.
/// op=1 (WAKE): wake up to `val` processes sleeping on `addr`.
///
/// addr must be 4-byte aligned and in user address space.
pub unsafe fn sys_futex(addr: u64, op: u64, val: u64, timeout_ms: u64) -> u64 {
    if addr == 0 || addr & 3 != 0 || addr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if !validate_user_buf(addr, 4) {
        return EFAULT;
    }

    match op {
        FUTEX_WAIT => {
            // Read the futex word atomically.
            let word_ptr = addr as *const u32;
            let current = core::ptr::read_volatile(word_ptr);

            // Spurious-safe: if someone already changed it, bail.
            if current != val as u32 {
                return u64::MAX - 11; // EAGAIN
            }

            // Block on this address.
            {
                let proc = SCHEDULER.current_process_mut();
                proc.state = crate::process::ProcessState::Blocked(
                    crate::process::BlockReason::FutexWait(addr),
                );
                // Set timeout deadline if requested.
                if timeout_ms > 0 {
                    let tsc_freq = crate::process::scheduler::tsc_frequency();
                    if tsc_freq > 0 {
                        let ticks_per_ms = tsc_freq / 1000;
                        let deadline = crate::cpu::tsc::read_tsc()
                            .saturating_add(timeout_ms.saturating_mul(ticks_per_ms));
                        proc.futex_deadline = deadline;
                    }
                }
            }
            core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            // Check if we timed out (state was set back to Ready by the timer ISR).
            0
        }
        FUTEX_WAKE => {
            let count = if val == 0 { 1 } else { val as u32 };
            crate::process::scheduler::wake_futex_waiters(addr, count) as u64
        }
        _ => EINVAL,
    }
}

// SYS_THREAD_CREATE (80) — spawn a thread in the caller's address space

/// `SYS_THREAD_CREATE(entry, stack_top, arg) → tid`
///
/// Creates a new thread sharing the caller's page tables.  The thread
/// starts at `entry` with `rdi = arg` and `rsp = stack_top`.  Caller
/// must allocate the stack (via SYS_MMAP) before calling this.
pub unsafe fn sys_thread_create(entry: u64, stack_top: u64, arg: u64) -> u64 {
    if entry == 0 || stack_top == 0 {
        return EINVAL;
    }
    if entry >= USER_ADDR_LIMIT || stack_top >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    // Stack must be 16-byte aligned (x86-64 ABI).
    if stack_top & 0xF != 0 {
        return EINVAL;
    }

    // Verify the first stack push target is mapped.
    {
        let proc = SCHEDULER.current_process_mut();
        let cr3 = proc.cr3;

        let ptm = crate::paging::table::PageTableManager { pml4_phys: cr3 };
        let check_addr = stack_top - 8;
        let page_addr = check_addr & !0xFFF;
        if ptm.translate(page_addr).is_none() {
            return EFAULT;
        }
    }

    match crate::process::spawn_user_thread(entry, stack_top, arg) {
        Ok(tid) => tid as u64,
        Err(_) => ENOMEM,
    }
}

// SYS_THREAD_EXIT (81) — terminate the calling thread

/// `SYS_THREAD_EXIT(code)` — exits the current thread.
///
/// Same as SYS_EXIT under the hood — the scheduler handles thread vs
/// process distinction via thread_group_leader.
pub unsafe fn sys_thread_exit(code: u64) -> u64 {
    crate::process::scheduler::exit_process(code as i32);
}

// SYS_THREAD_JOIN (82) — wait for a thread to finish

/// `SYS_THREAD_JOIN(tid) → exit_code`
///
/// Blocks until the thread with `tid` exits.  Reuses the wait-for-child
/// mechanism since threads are just processes with shared CR3.
pub unsafe fn sys_thread_join(tid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(tid as u32)
}

// SYS_SIGRETURN (83) — restore context after signal handler

/// `SYS_SIGRETURN() → 0`
///
/// Restores the saved pre-signal context.  Must be called by user signal
/// handlers when they are done.  If called outside a signal handler, returns
/// -EINVAL.
pub unsafe fn sys_sigreturn() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    if !proc.in_signal_handler {
        return EINVAL;
    }
    proc.context = proc.saved_signal_context;
    proc.fpu_state = proc.saved_signal_fpu;
    proc.in_signal_handler = false;
    proc.context.rax
}

// SYS_MOUSE_READ (84) — read accumulated relative mouse state

/// `SYS_MOUSE_READ() → packed(dx:i16 | dy:i16 | buttons:u8)`
///
/// Returns accumulated relative motion since last call.
/// Bits [15:0] = dx (i16), [31:16] = dy (i16), [39:32] = buttons.
pub unsafe fn sys_mouse_read() -> u64 {
    let (dx, dy, buttons) = crate::mouse::drain();
    let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
    let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
    (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32)
}
