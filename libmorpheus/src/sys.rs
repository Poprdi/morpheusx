//! System information — sysinfo, syslog.

use crate::is_error;
use crate::raw::*;

pub const SYSINFO_MAX_CPUS: usize = 16;

/// Matches kernel `SysInfo` layout exactly. Populated by `sysinfo()`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SysInfo {
    pub total_mem: u64,
    pub free_mem: u64,
    pub num_procs: u32,
    pub cpu_count: u32,
    pub uptime_ticks: u64,
    /// TSC frequency in Hz.
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub sched_ticks: u64,
    /// TSC cycles spent halted in HLT idle. Inter-poll delta / `uptime_ticks` delta
    /// gives idle fraction; subtract from 1.0 for utilization.
    pub idle_tsc: u64,
    /// Per-core halted TSC cycles. Valid entries are `0..cpu_count`.
    pub per_core_idle_tsc: [u64; SYSINFO_MAX_CPUS],
}

impl SysInfo {
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

    pub fn uptime_ms(&self) -> u64 {
        if self.tsc_freq == 0 {
            return 0;
        }
        (self.uptime_ticks as u128 * 1000 / self.tsc_freq as u128) as u64
    }
}

pub fn sysinfo(info: &mut SysInfo) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_SYSINFO, info as *mut SysInfo as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Write to the kernel serial log, bypassing console/window system.
pub fn syslog(msg: &str) {
    if msg.is_empty() {
        return;
    }
    unsafe {
        syscall2(SYS_SYSLOG, msg.as_ptr() as u64, msg.len() as u64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum SystemControlMode {
    RebootGraceful = SYSCTL_REBOOT_GRACEFUL,
    RebootForce = SYSCTL_REBOOT_FORCE,
    ShutdownGraceful = SYSCTL_SHUTDOWN_GRACEFUL,
    ShutdownForce = SYSCTL_SHUTDOWN_FORCE,
    ShutdownPanic = SYSCTL_SHUTDOWN_PANIC,
}

/// Does not return on success.
pub fn system_control(mode: SystemControlMode) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_SYSTEM_CONTROL, mode as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn reboot(force: bool) -> Result<(), u64> {
    let mode = if force {
        SystemControlMode::RebootForce
    } else {
        SystemControlMode::RebootGraceful
    };
    system_control(mode)
}

pub fn shutdown(force: bool) -> Result<(), u64> {
    let mode = if force {
        SystemControlMode::ShutdownForce
    } else {
        SystemControlMode::ShutdownGraceful
    };
    system_control(mode)
}

pub fn shutdown_panic() -> Result<(), u64> {
    system_control(SystemControlMode::ShutdownPanic)
}

/// Fill `buf` with hardware random bytes. Returns the number written (may be
/// short on transient entropy starvation). `Err(ENOSYS)` if the platform has
/// no RNG. Linux-`getrandom`-shaped; the seed source for std's HashMap etc.
pub fn getrandom(buf: &mut [u8]) -> Result<usize, u64> {
    getrandom_flags(buf, 0)
}

/// `getrandom` with raw flags (e.g. `GRND_NONBLOCK`).
pub fn getrandom_flags(buf: &mut [u8], flags: u64) -> Result<usize, u64> {
    if buf.is_empty() {
        return Ok(0);
    }
    let ret = unsafe {
        syscall3(
            SYS_GETRANDOM,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            flags,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}
