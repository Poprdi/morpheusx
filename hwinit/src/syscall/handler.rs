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
