
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
    /// Total scheduler timer ticks since boot.
    pub sched_ticks: u64,
    /// Total TSC cycles the kernel has spent halted in HLT idle since boot.
    /// Use with `uptime_ticks` delta to compute true system-wide idle fraction:
    ///   idle_pct = (idle_tsc_delta / uptime_ticks_delta) * 100
    pub idle_tsc: u64,
}

/// `SYS_SYSINFO(buf_ptr) → 0`
///
/// Fills `buf_ptr` with a `SysInfo` struct.
pub unsafe fn sys_sysinfo(buf_ptr: u64) -> u64 {
    let size = core::mem::size_of::<SysInfo>() as u64;
    if !validate_user_buf(buf_ptr, size) {
        return EFAULT;
    }

    // heap_stats acquires HEAP lock. global_registry acquires GLOBAL_REGISTRY.
    // heap growth path does HEAP → GLOBAL_REGISTRY. acquiring GLOBAL_REGISTRY
    // first then HEAP would be an ABBA inversion. so: heap first.
    let (heap_total, heap_used, heap_free) = crate::heap::heap_stats().unwrap_or((0, 0, 0));
    let tsc_freq = crate::process::scheduler::tsc_frequency();

    let (total_mem, free_mem) = {
        let registry = crate::memory::global_registry();
        (registry.total_memory(), registry.free_memory())
    };

    let info = SysInfo {
        total_mem,
        free_mem,
        num_procs: SCHEDULER.live_count(),
        _pad0: 0,
        uptime_ticks: crate::cpu::tsc::read_tsc(),
        tsc_freq,
        heap_total: heap_total as u64,
        heap_used: heap_used as u64,
        heap_free: heap_free as u64,
        sched_ticks: SCHEDULER.tick_count() as u64,
        idle_tsc: crate::process::scheduler::idle_tsc_total(),
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
    let mut _vfs_guard = match vfs_lock() {
        Some(g) => g,
        None => return ENOSYS,
    };
    let fs = &mut *_vfs_guard.fs;

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
    // drop the registry guard before spawn_user_process — load_elf64 needs
    // GLOBAL_REGISTRY for page table allocation. holding it here deadlocks.
    let pages_needed = file_size.div_ceil(4096) as u64;
    let buf_phys = {
        let mut registry = crate::memory::global_registry_mut();
        match registry.allocate_pages(
            crate::memory::AllocateType::AnyPages,
            crate::memory::MemoryType::Allocated,
            pages_needed,
        ) {
            Ok(addr) => addr,
            Err(_) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                return ENOMEM;
            }
        }
    }; // registry guard dropped here

    // Read entire file into the buffer.
    let buf = core::slice::from_raw_parts_mut(buf_phys as *mut u8, file_size);
    let bytes_read =
        match morpheus_helix::vfs::vfs_read(&mut fs.device, &fs.mount_table, fd_table, fd, buf) {
            Ok(n) => n,
            Err(_) => {
                let _ = morpheus_helix::vfs::vfs_close(fd_table, fd);
                let _ = crate::memory::global_registry_mut().free_pages(buf_phys, pages_needed);
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
            let _ = crate::memory::global_registry_mut().free_pages(buf_phys, pages_needed);
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
    // GLOBAL_REGISTRY is NOT held here — spawn_user_process → load_elf64
    // needs it for page table allocation.
    let elf_data = &buf[..bytes_read];
    let result = crate::process::scheduler::spawn_user_process(
        name,
        elf_data,
        &arg_blob[..blob_len],
        arg_count,
        true, // inherit fds from parent
    );

    // Free the temporary buffer. re-acquire registry for the free call.
    let _ = crate::memory::global_registry_mut().free_pages(buf_phys, pages_needed);

    match result {
        Ok(pid) => pid as u64,
        Err(_) => ENOMEM,
    }
}
