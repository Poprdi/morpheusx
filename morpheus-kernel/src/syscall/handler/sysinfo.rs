// ps/sigaction/priority/cpuid/rdtsc/boot_log/memmap.

use super::common::*;
use crate::hal;
use crate::process::signals::Signal;
use crate::process::ProcessState;
use crate::schedular::SCHEDULER;

/// Must match `libmorpheus::process::PsEntry` byte-for-byte.
/// state: 0 Ready, 1 Running, 2 Blocked, 3 Zombie, 4 Terminated.
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,
    pub priority: u32,
    pub cpu_ticks: u64,
    /// Running-TSC only; HLT idle excluded for PID 0.
    pub cpu_tsc: u64,
    pub pages_alloc: u64,
    pub name: [u8; 32],
}

pub unsafe fn sys_ps(buf_ptr: u64, max_count: u64) -> u64 {
    if max_count == 0 || buf_ptr == 0 {
        return SCHEDULER.live_count() as u64;
    }
    let entry_size = core::mem::size_of::<PsEntry>() as u64;
    let total_size = max_count.saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total_size) {
        return EFAULT;
    }

    let max = max_count.min(64) as usize;
    let mut infos = [crate::schedular::ProcessInfo::zeroed(); 64];
    let count = SCHEDULER.snapshot_processes(&mut infos[..max]);

    let out = core::slice::from_raw_parts_mut(buf_ptr as *mut PsEntry, max);
    for i in 0..count {
        let pi = &infos[i];
        let state_u32 = match pi.state {
            ProcessState::Ready => 0,
            ProcessState::Running => 1,
            ProcessState::Blocked(_) => 2,
            ProcessState::Zombie => 3,
            ProcessState::Terminated => 4,
        };
        let mut entry = PsEntry {
            pid: pi.pid,
            ppid: 0, // ProcessInfo lacks ppid; leave 0.
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

/// `handler`: 0 = SIG_DFL, 1 = SIG_IGN, otherwise user fn address.
pub unsafe fn sys_sigaction(signum: u64, handler: u64) -> u64 {
    let sig = match Signal::from_u8(signum as u8) {
        Some(s) => s,
        None => return EINVAL,
    };
    if matches!(sig, Signal::SIGKILL | Signal::SIGSTOP) {
        return EINVAL;
    }

    let proc = SCHEDULER.current_process_mut();
    let sig_idx = signum as usize;
    let old = proc.signal_handlers[sig_idx];
    proc.signal_handlers[sig_idx] = handler;
    old
}

/// pid=0 means current. priority 0..255 (0=highest).
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

#[repr(C)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// LD25: on non-x86 HALs `Cpu::cpuid` returns `[0; 4]`, which we surface as
/// ENOSYS so userspace doesn't get phantom-zero feature bits.
pub unsafe fn sys_cpuid(leaf: u64, subleaf: u64, result_ptr: u64) -> u64 {
    let size = core::mem::size_of::<CpuidResult>() as u64;
    if !validate_user_buf(result_ptr, size) {
        return EFAULT;
    }

    let regs = hal().cpu().cpuid(leaf as u32, subleaf as u32);

    // All-zero sentinel from the ARM HAL stub → ENOSYS.
    if regs == [0u32; 4] {
        return ENOSYS;
    }

    core::ptr::write(
        result_ptr as *mut CpuidResult,
        CpuidResult {
            eax: regs[0],
            ebx: regs[1],
            ecx: regs[2],
            edx: regs[3],
        },
    );
    0
}

#[repr(C)]
pub struct TscResult {
    pub tsc: u64,
    pub frequency: u64,
}

/// Writes `TscResult` if `result_ptr` is set; returns the TSC.
pub unsafe fn sys_rdtsc(result_ptr: u64) -> u64 {
    let tsc = hal().timer().read_tsc();
    let freq = crate::schedular::tsc_frequency();

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

/// `buf_len == 0` returns total log size without copying.
///
/// The HAL doesn't expose a boot log accessor today (the `crate::serial::boot_log()`
/// shim returns ""), so we honor the "size query" semantics by returning 0
/// and copy nothing.
pub unsafe fn sys_boot_log(buf_ptr: u64, buf_len: u64) -> u64 {
    if buf_len == 0 {
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemmapEntry {
    pub phys_start: u64,
    pub num_pages: u64,
    pub mem_type: u32,
    pub _pad: u32,
}

/// `buf_ptr == 0` returns total entry count.
pub unsafe fn sys_memmap(buf_ptr: u64, max_entries: u64) -> u64 {
    let phys = hal().phys();

    // Count first.
    let mut total = 0usize;
    phys.for_each_descriptor(&mut |_| total += 1);

    if buf_ptr == 0 || max_entries == 0 {
        return total as u64;
    }

    let entry_size = core::mem::size_of::<MemmapEntry>() as u64;
    let total_size = max_entries.saturating_mul(entry_size);
    if !validate_user_buf(buf_ptr, total_size) {
        return EFAULT;
    }

    let out = core::slice::from_raw_parts_mut(buf_ptr as *mut MemmapEntry, max_entries as usize);
    let mut idx = 0usize;
    phys.for_each_descriptor(&mut |desc| {
        if idx < max_entries as usize {
            out[idx] = MemmapEntry {
                phys_start: desc.phys_start,
                num_pages: desc.num_pages,
                mem_type: desc.mem_type as u32,
                _pad: 0,
            };
            idx += 1;
        }
    });

    idx as u64
}
