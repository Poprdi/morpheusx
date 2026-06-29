//! Process management — exit, yield, signal, getpid, spawn, Command.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{self, Error};
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

pub fn sleep(millis: u64) {
    unsafe {
        syscall1(SYS_SLEEP, millis);
    }
}

/// Block until child exits; returns its exit code (or `128 + signal` if killed).
pub fn wait(pid: u32) -> Result<i32, u64> {
    use morpheus_foundation::flags::P_PID;
    use morpheus_foundation::types::WaitStatus;
    let mut ws = WaitStatus::default();
    let ret = unsafe {
        syscall4(
            SYS_WAIT,
            P_PID,
            pid as u64,
            &mut ws as *mut WaitStatus as u64,
            0,
        )
    };
    if is_error(ret) {
        return Err(ret);
    }
    Ok(decode_wstatus(ws.wstatus))
}

/// Collapse a Linux wstatus word into the single exit code the legacy callers
/// expect: the exit status for a normal exit, else `128 + signal`.
fn decode_wstatus(s: i32) -> i32 {
    use morpheus_foundation::types::WaitStatus;
    if WaitStatus::exited(s) {
        WaitStatus::exit_status(s)
    } else if WaitStatus::signaled(s) {
        128 + WaitStatus::term_sig(s)
    } else {
        0
    }
}

/// `Ok(None)` means still running.
pub fn try_wait(pid: u32) -> Result<Option<i32>, u64> {
    let ret = unsafe { syscall1(SYS_TRY_WAIT, pid as u64) };
    if ret == morpheus_foundation::errno::EAGAIN {
        Ok(None)
    } else if is_error(ret) {
        Err(ret)
    } else {
        Ok(Some(ret as i32))
    }
}

/// Build a versioned `SpawnArgs` for SYS_SPAWN; the child inherits our fds
/// (minus O_CLOEXEC) with no file_actions.
fn make_spawn_args(path: &str, descs: &[[u64; 2]]) -> morpheus_foundation::types::SpawnArgs {
    use morpheus_foundation::types::{SpawnArgs, SpawnFileAction};
    SpawnArgs {
        version: 1,
        struct_size: core::mem::size_of::<SpawnArgs>() as u16,
        path_ptr: path.as_ptr() as u64,
        path_len: path.len() as u64,
        argv_ptr: if descs.is_empty() { 0 } else { descs.as_ptr() as u64 },
        argc: descs.len() as u64,
        fa_stride: core::mem::size_of::<SpawnFileAction>() as u32,
        ..SpawnArgs::default()
    }
}

/// Spawn ELF at `path`; returns child PID.
pub fn spawn(path: &str) -> Result<u32, u64> {
    let sa = make_spawn_args(path, &[]);
    let ret = unsafe {
        syscall1(
            SYS_SPAWN,
            &sa as *const morpheus_foundation::types::SpawnArgs as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u32)
    }
}

/// Max 16 args. Child inherits our FDs.
pub fn spawn_with_args(path: &str, args: &[&str]) -> Result<u32, u64> {
    // argv descriptor array: [ptr, len] pairs on the stack.
    let mut descs = [[0u64; 2]; 16];
    let count = args.len().min(16);
    for i in 0..count {
        descs[i][0] = args[i].as_ptr() as u64;
        descs[i][1] = args[i].len() as u64;
    }
    let sa = make_spawn_args(path, &descs[..count]);
    let ret = unsafe {
        syscall1(
            SYS_SPAWN,
            &sa as *const morpheus_foundation::types::SpawnArgs as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u32)
    }
}

// `PsEntry` (+ `zeroed`/`name_str`) is canonical in morpheus-foundation.
pub use morpheus_foundation::types::PsEntry;

pub fn ps_count() -> u32 {
    unsafe { syscall2(SYS_PS, 0, 0) as u32 }
}

/// Returns entries written.
pub fn ps(entries: &mut [PsEntry]) -> usize {
    let ret = unsafe { syscall2(SYS_PS, entries.as_mut_ptr() as u64, entries.len() as u64) };
    if is_error(ret) {
        0
    } else {
        ret as usize
    }
}

// Signal numbers are canonical in morpheus-foundation; re-export keeps the
// `process::signal::SIG*` path while single-sourcing the values.
pub use morpheus_foundation::flags::signal;

/// sigaction. handler=0 → default, handler=1 → ignore.
pub fn sigaction(signum: u8, handler: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall2(SYS_SIGACTION, signum as u64, handler) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Restore pre-signal context. MUST be called at end of every `sigaction()` handler;
/// otherwise the handler's return address is 0 and faults on return.
pub fn sigreturn() {
    unsafe {
        syscall0(SYS_SIGRETURN);
    }
}

/// pid=0 means self. 0=highest, 255=lowest.
pub fn setpriority(pid: u32, priority: u8) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_SETPRIORITY, pid as u64, priority as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// pid=0 means self.
pub fn getpriority(pid: u32) -> Result<u8, u64> {
    let ret = unsafe { syscall1(SYS_GETPRIORITY, pid as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u8)
    }
}

/// Returns `(read_fd, write_fd)`.
pub fn pipe() -> Result<(u32, u32), u64> {
    let mut fds = [0u32; 2];
    let ret = unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok((fds[0], fds[1]))
    }
}

/// Closes `new_fd` first if open.
pub fn dup2(old_fd: u32, new_fd: u32) -> Result<u32, u64> {
    let ret = unsafe { syscall2(SYS_DUP2, old_fd as u64, new_fd as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as u32)
    }
}

/// Foreground process receives Ctrl+C as SIGINT.
pub fn set_foreground(pid: u32) {
    unsafe {
        syscall1(SYS_SET_FG, pid as u64);
    }
}

pub fn argc() -> usize {
    let ret = unsafe { syscall2(SYS_GETARGS, 0, 0) };
    ret as usize
}

/// Writes null-separated args. Use `parse_args()` to split.
pub fn getargs(buf: &mut [u8]) -> usize {
    let ret = unsafe { syscall2(SYS_GETARGS, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if is_error(ret) {
        0
    } else {
        ret as usize
    }
}

/// Returns number of args written into `out`.
pub fn parse_args<'a>(buf: &'a [u8], out: &mut [&'a str]) -> usize {
    let mut count = 0;
    let mut start = 0;
    for i in 0..buf.len() {
        if buf[i] == 0 {
            if i > start && count < out.len() {
                if let Ok(s) = core::str::from_utf8(&buf[start..i]) {
                    out[count] = s;
                    count += 1;
                }
            }
            start = i + 1;
        }
    }
    // Last arg may not have trailing null.
    if start < buf.len() && count < out.len() {
        if let Ok(s) = core::str::from_utf8(&buf[start..]) {
            if !s.is_empty() {
                out[count] = s;
                count += 1;
            }
        }
    }
    count
}

/// Process builder.
pub struct Command {
    path: String,
    args: Vec<String>,
}

impl Command {
    pub fn new(path: &str) -> Self {
        Self {
            path: String::from(path),
            args: Vec::new(),
        }
    }

    pub fn arg(&mut self, arg: &str) -> &mut Self {
        self.args.push(String::from(arg));
        self
    }

    pub fn args(&mut self, args: &[&str]) -> &mut Self {
        for a in args {
            self.args.push(String::from(*a));
        }
        self
    }

    pub fn spawn_pid(&self) -> error::Result<u32> {
        if self.args.is_empty() {
            spawn(&self.path).map_err(Error::from_raw)
        } else {
            let refs: Vec<&str> = self.args.iter().map(|s| s.as_str()).collect();
            spawn_with_args(&self.path, &refs).map_err(Error::from_raw)
        }
    }

    /// Spawn and wait. Returns exit code.
    pub fn status(&self) -> error::Result<i32> {
        let pid = self.spawn_pid()?;
        wait(pid).map_err(Error::from_raw)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExitStatus {
    code: i32,
}

impl ExitStatus {
    pub fn new(code: i32) -> Self {
        Self { code }
    }

    pub fn code(&self) -> i32 {
        self.code
    }

    pub fn success(&self) -> bool {
        self.code == 0
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "exit code: {}", self.code)
    }
}
