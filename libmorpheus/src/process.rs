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

// ═══════════════════════════════════════════════════════════════════════════
// PROCESS LISTING (SYS_PS)
// ═══════════════════════════════════════════════════════════════════════════

/// Process table entry returned by `ps()`.
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    /// 0=Ready, 1=Running, 2=Blocked, 3=Zombie, 4=Terminated
    pub state: u32,
    pub priority: u32,
    pub cpu_ticks: u64,
    pub pages_alloc: u64,
    /// NUL-terminated process name.
    pub name: [u8; 32],
}

impl PsEntry {
    pub const fn zeroed() -> Self {
        Self {
            pid: 0,
            ppid: 0,
            state: 0,
            priority: 0,
            cpu_ticks: 0,
            pages_alloc: 0,
            name: [0u8; 32],
        }
    }

    /// Get the process name as a string slice.
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// Get the number of live processes.
pub fn ps_count() -> u32 {
    unsafe { syscall2(SYS_PS, 0, 0) as u32 }
}

/// List all processes.  Returns the number of entries written.
pub fn ps(entries: &mut [PsEntry]) -> usize {
    let ret = unsafe { syscall2(SYS_PS, entries.as_mut_ptr() as u64, entries.len() as u64) };
    if is_error(ret) {
        0
    } else {
        ret as usize
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SIGNAL HANDLING (SYS_SIGACTION)
// ═══════════════════════════════════════════════════════════════════════════

/// Well-known signal numbers.
pub mod signal {
    pub const SIGINT: u8 = 2;
    pub const SIGKILL: u8 = 9;
    pub const SIGSEGV: u8 = 11;
    pub const SIGTERM: u8 = 15;
    pub const SIGCHLD: u8 = 17;
    pub const SIGCONT: u8 = 18;
    pub const SIGSTOP: u8 = 19;
}

/// Register a signal handler.
///
/// `handler` = 0 → default action, `handler` = 1 → ignore.
/// Returns the previous handler address.
pub fn sigaction(signum: u8, handler: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall2(SYS_SIGACTION, signum as u64, handler) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PRIORITY (SYS_SETPRIORITY / SYS_GETPRIORITY)
// ═══════════════════════════════════════════════════════════════════════════

/// Set the scheduling priority of a process.
///
/// `pid` = 0 means current process.
/// `priority`: 0 (highest) to 255 (lowest).
pub fn setpriority(pid: u32, priority: u8) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_SETPRIORITY, pid as u64, priority as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Get the scheduling priority of a process.
///
/// `pid` = 0 means current process.
pub fn getpriority(pid: u32) -> Result<u8, u64> {
    let ret = unsafe { syscall1(SYS_GETPRIORITY, pid as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u8)
    }
}
