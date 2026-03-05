
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
    /// Accumulated TSC cycles actively running (HLT idle excluded for PID 0).
    pub cpu_tsc: u64,
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
            cpu_tsc: pi.cpu_tsc,
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
