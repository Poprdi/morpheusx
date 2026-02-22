//! Process management — exit, yield, signal, getpid.

use crate::raw::*;
use crate::is_error;

pub fn exit(code: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, code as u64); }
    loop { core::hint::spin_loop(); }
}

pub fn getpid() -> u32 {
    unsafe { syscall0(SYS_GETPID) as u32 }
}

pub fn yield_cpu() {
    unsafe { syscall0(SYS_YIELD); }
}

pub fn kill(pid: u32, signal: u8) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_KILL, pid as u64, signal as u64) };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn sleep(ticks: u64) {
    unsafe { syscall1(SYS_SLEEP, ticks); }
}
