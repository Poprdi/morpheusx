// Process/system info syscalls: clock/sysinfo/getppid/spawn.

use super::common::*;
use crate::hal;
use crate::schedular::SCHEDULER;
use morpheus_foundation::PAGE_SIZE;
use morpheus_hal_api::{AllocKind, MemoryType};

const SYSINFO_MAX_CPUS: usize = 16;

/// Monotonic ns from TSC. 0 if TSC isn't calibrated.
pub unsafe fn sys_clock() -> u64 {
    let freq = crate::schedular::tsc_frequency();
    if freq == 0 {
        return 0;
    }
    let tsc = hal().timer().read_tsc();
    // 128-bit intermediate avoids overflow at high TSC values.
    let nanos_wide = (tsc as u128) * 1_000_000_000u128 / (freq as u128);
    nanos_wide as u64
}

/// SYS_SET_THREAD_POINTER: set the calling thread's TLS base (x86 FS base).
/// Validates the canonical-user range (which also makes the `wrmsr` safe),
/// records it on the Process for per-switch restore, and applies it live for
/// the current thread. `tp == 0` clears TLS.
pub unsafe fn sys_set_thread_pointer(tp: u64) -> u64 {
    // Canonical lower-half: both the user/kernel boundary and wrmsr-#GP safety
    // (AMD64 Vol 2 §5.3). 0 is allowed and falls inside the range.
    if tp >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    SCHEDULER.current_process_mut().tls_base = tp;
    hal().cpu().set_user_tls_base(tp);
    0
}

/// Must match `libmorpheus::sys::SysInfo` byte-for-byte.
#[repr(C)]
pub struct SysInfo {
    pub total_mem: u64,
    pub free_mem: u64,
    pub num_procs: u32,
    pub cpu_count: u32,
    pub uptime_ticks: u64,
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub sched_ticks: u64,
    /// Total HLT TSC across all cores. Pair with `uptime_ticks` delta for
    /// idle_pct = idle_tsc_delta / uptime_ticks_delta.
    pub idle_tsc: u64,
    /// HLT TSC per core; valid for indices `0..cpu_count`.
    pub per_core_idle_tsc: [u64; SYSINFO_MAX_CPUS],
}

pub unsafe fn sys_sysinfo(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<SysInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }

    // Heap stats lives in the HAL today (host crate hands its allocator's
    // metrics back through `phys()` totals).
    let phys = hal().phys();
    let total_mem = phys.total_memory();
    let free_mem = phys.free_memory();
    let used = phys.allocated_memory();

    let tsc_freq = crate::schedular::tsc_frequency();

    let cpu_count = hal().smp().cpu_count();
    let mut per_core_idle_tsc = [0u64; SYSINFO_MAX_CPUS];
    crate::schedular::sample_per_core_idle_tsc(&mut per_core_idle_tsc);

    let info = SysInfo {
        total_mem,
        free_mem,
        num_procs: SCHEDULER.live_count(),
        cpu_count,
        uptime_ticks: hal().timer().read_tsc(),
        tsc_freq,
        heap_total: total_mem,
        heap_used: used,
        heap_free: free_mem,
        sched_ticks: SCHEDULER.tick_count() as u64,
        idle_tsc: crate::schedular::idle_tsc_total(),
        per_core_idle_tsc,
    };

    let dst = buf_ptr as *mut SysInfo;
    core::ptr::write(dst, info);
    0
}

pub unsafe fn sys_getppid() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    proc.parent_pid as u64
}

/// `argv_ptr`: array of `[ptr, len]` pairs (`argc` of them).
pub unsafe fn sys_spawn(path_ptr: u64, path_len: u64, argv_ptr: u64, argc: u64) -> u64 {
    let path = match user_path(path_ptr, path_len) {
        Some(p) => p,
        None => return EINVAL,
    };

    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

    let fd_table = SCHEDULER.current_fd_table_mut();
    let ts = hal().timer().read_tsc();

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

    let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, path) {
        Ok(s) => s,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
            return EIO;
        },
    };

    let file_size = stat.size as usize;
    if file_size == 0 || file_size > 4 * 1024 * 1024 {
        let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
        return EINVAL;
    }

    // Drop registry before spawn_user_process — load_elf64 reacquires it.
    let pages_needed = file_size.div_ceil(PAGE_SIZE as usize) as u64;
    let buf_phys =
        match hal()
            .phys()
            .allocate_pages(AllocKind::AnyPages, MemoryType::Allocated, pages_needed)
        {
            Ok(addr) => addr,
            Err(_) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                return ENOMEM;
            },
        };

    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, file_size);
    let bytes_read =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
            Ok(n) => n,
            Err(_) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                let _ = hal().phys().free_pages(buf_phys, pages_needed);
                return EIO;
            },
        };

    let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);

    let name = path.rsplit('/').next().unwrap_or(path);

    // Pack argv into a NUL-separated blob.
    let mut arg_blob = [0u8; 256];
    let mut blob_len: usize = 0;
    let mut arg_count: u8 = 0;
    if argc > 0 && argc <= 16 && argv_ptr != 0 {
        let argv_size = argc.saturating_mul(16); // 2×u64 per entry
        if !validate_user_buf(argv_ptr, argv_size) {
            let _ = hal().phys().free_pages(buf_phys, pages_needed);
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
            arg_blob[blob_len] = 0;
            blob_len += 1;
            arg_count += 1;
        }
    }

    let elf_data = &buf[..bytes_read];
    let result = crate::schedular::spawn_user_process(
        name,
        elf_data,
        &arg_blob[..blob_len],
        arg_count,
        true,
    );

    let _ = hal().phys().free_pages(buf_phys, pages_needed);

    match result {
        Ok(pid) => pid as u64,
        Err(_) => ENOMEM,
    }
}
