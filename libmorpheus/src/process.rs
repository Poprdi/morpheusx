//! Process management — exit, yield, signal, getpid, spawn.

use crate::is_error;
use crate::raw::*;

pub fn exit(code: i32) -> ! {
    unsafe {
        syscall1(SYS_EXIT, code as u64);
    }
    loop {
        core::hint::spin_loop();
    }
}

pub fn getpid() -> u32 {
    unsafe { syscall0(SYS_GETPID) as u32 }
}

/// Get the parent process ID.
pub fn getppid() -> u32 {
    unsafe { syscall0(SYS_GETPPID) as u32 }
}

pub fn yield_cpu() {
    unsafe {
        syscall0(SYS_YIELD);
    }
}

pub fn kill(pid: u32, signal: u8) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_KILL, pid as u64, signal as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Sleep for `millis` milliseconds.
pub fn sleep(millis: u64) {
    unsafe {
        syscall1(SYS_SLEEP, millis);
    }
}

/// Wait for a child process to exit, returning its exit code.
pub fn wait(pid: u32) -> Result<i32, u64> {
    let ret = unsafe { syscall1(SYS_WAIT, pid as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as i32)
    }
}

/// Spawn a child process from an ELF binary path in the filesystem.
///
/// Returns the child PID on success.
pub fn spawn(path: &str) -> Result<u32, u64> {
    let ret = unsafe { syscall2(SYS_SPAWN, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u32)
    }
}
