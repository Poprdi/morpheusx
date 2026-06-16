//! System information — sysinfo, syslog.

use crate::is_error;
use crate::raw::*;

// `SysInfo` (+ `SYSINFO_MAX_CPUS`, `zeroed`/`uptime_ms`) is canonical in
// morpheus-foundation — single source of truth across the syscall seam.
pub use morpheus_foundation::types::{SysInfo, SYSINFO_MAX_CPUS};

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
/// no RNG. Linux-`getrandom`-shaped.
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
