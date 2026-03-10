//! System information — sysinfo, syslog.

use crate::is_error;
use crate::raw::*;

/// Maximum number of per-core utilization counters exported through `SysInfo`.
pub const SYSINFO_MAX_CPUS: usize = 16;

/// System information struct — matches the kernel's `SysInfo` layout exactly.
///
/// Populated by `sysinfo()`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SysInfo {
    /// Total physical memory in bytes.
    pub total_mem: u64,
    /// Free physical memory in bytes.
    pub free_mem: u64,
    /// Number of live processes.
    pub num_procs: u32,
    /// Number of online CPUs known to the kernel.
    pub cpu_count: u32,
    /// TSC ticks since boot.
    pub uptime_ticks: u64,
    /// TSC frequency in Hz (divide `uptime_ticks` by this for seconds).
    pub tsc_freq: u64,
    /// Kernel heap: total bytes.
    pub heap_total: u64,
    /// Kernel heap: used bytes.
    pub heap_used: u64,
    /// Kernel heap: free bytes.
    pub heap_free: u64,
    /// Total scheduler timer ticks since boot.
    pub sched_ticks: u64,
    /// Total TSC cycles the kernel has spent halted in HLT idle since boot.
    /// Compute the inter-poll delta and divide by the `uptime_ticks` delta to
    /// get the idle fraction; subtract from 1.0 for true CPU utilization.
    pub idle_tsc: u64,
    /// Per-core halted TSC cycles since boot (indexed by core id).
    /// Valid entries are `0..cpu_count`; remaining slots are zeroed.
    pub per_core_idle_tsc: [u64; SYSINFO_MAX_CPUS],
}

impl SysInfo {
    /// Create a zeroed SysInfo (for passing to `sysinfo()`).
    pub const fn zeroed() -> Self {
        Self {
            total_mem: 0,
            free_mem: 0,
            num_procs: 0,
            cpu_count: 0,
            uptime_ticks: 0,
            tsc_freq: 0,
            heap_total: 0,
            heap_used: 0,
            heap_free: 0,
            sched_ticks: 0,
            idle_tsc: 0,
            per_core_idle_tsc: [0; SYSINFO_MAX_CPUS],
        }
    }

    /// Uptime in milliseconds.
    pub fn uptime_ms(&self) -> u64 {
        if self.tsc_freq == 0 {
            return 0;
        }
        (self.uptime_ticks as u128 * 1000 / self.tsc_freq as u128) as u64
    }
}

/// Fill a `SysInfo` struct with current system information.
pub fn sysinfo(info: &mut SysInfo) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_SYSINFO, info as *mut SysInfo as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Write a message to the kernel serial log.
///
/// This bypasses the console/window system and writes directly to the
/// serial port.  Useful for debugging.
pub fn syslog(msg: &str) {
    if msg.is_empty() {
        return;
    }
    unsafe {
        syscall2(SYS_SYSLOG, msg.as_ptr() as u64, msg.len() as u64);
    }
}
