// Syscall handlers. args come in as u64, result goes out as u64.
// Negative = errno, 0 = ok, positive = data. MS x64 ABI.

use crate::process::scheduler::{exit_process, SCHEDULER};
use crate::process::signals::Signal;
use crate::serial::puts;
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

// SYS_EXIT — terminate the current process

/// `SYS_EXIT(code: i32)` — terminate the calling process.
///
/// Never returns.
pub unsafe fn sys_exit(code: u64) -> u64 {
    exit_process(code as i32);
}

// SYS_WRITE — fd-aware write (serial for fd 1/2, VFS for fd >= 3)

/// `SYS_WRITE(fd, ptr, len)` — write bytes.
///
/// For fds 0, 1, 2: if the fd has been redirected via dup2 (e.g. to a pipe
/// or file), writes to that target.  Otherwise fd 1/2 fall through to the
/// serial console, and fd 0 returns EBADF.
///
/// fd >= 3: pipe write or VFS file write.
pub unsafe fn sys_write(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }

    // For ALL fds (including 0, 1, 2): check the fd_table first.
    // If the fd has been explicitly opened/redirected (pipe, file), use it.
    {
        let fd_table = SCHEDULER.current_fd_table_mut();
        if let Ok(desc) = fd_table.get(fd as usize) {
            if desc.flags & O_PIPE_WRITE != 0 {
                let pipe_idx = desc.mount_idx;
                let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);
                if crate::pipe::pipe_readers(pipe_idx) == 0 {
                    return EPIPE;
                }
                let n = crate::pipe::pipe_write(pipe_idx, data);
                crate::process::scheduler::wake_pipe_readers(pipe_idx);
                return n as u64;
            }
            // Regular file — fall through to VFS write.
            let fs = match morpheus_helix::vfs::global::fs_global_mut() {
                Some(fs) => fs,
                None => return ENOSYS,
            };
            let fd_table = SCHEDULER.current_fd_table_mut();
            let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            let ts = crate::cpu::tsc::read_tsc();
            return match morpheus_helix::vfs::vfs_write(
                &mut fs.device,
                &mut fs.mount_table,
                fd_table,
                fd as usize,
                data,
                ts,
            ) {
                Ok(n) => n as u64,
                Err(e) => helix_err_to_errno(e),
            };
        }
    }

    // fd_table has no entry for this fd — legacy fallback.
    match fd {
        1 | 2 => {
            let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            // Capture output for the desktop shell widget
            crate::stdout::push(bytes);
            if let Ok(s) = core::str::from_utf8(bytes) {
                puts(s);
                len
            } else {
                for &b in bytes {
                    crate::serial::putc(b);
                }
                len
            }
        }
        _ => EBADF,
    }
}

// SYS_READ — fd-aware read (VFS for fd >= 3)

/// `SYS_READ(fd, ptr, len)` — read bytes.
///
/// For fds 0, 1, 2: if the fd has been redirected via dup2 (e.g. to a pipe
/// or file), reads from that target.  Otherwise fd 0 falls through to the
/// kernel keyboard ring buffer, and fd 1/2 return EBADF.
///
/// fd >= 3: pipe read or VFS file read.
pub unsafe fn sys_read(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }

    // For ALL fds (including 0, 1, 2): check the fd_table first.
    // If the fd has been explicitly opened/redirected (pipe, file), use it.
    {
        let fd_table = SCHEDULER.current_fd_table_mut();
        if let Ok(desc) = fd_table.get(fd as usize) {
            if desc.flags & O_PIPE_READ != 0 {
                let pipe_idx = desc.mount_idx;
                let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
                return sys_pipe_read_blocking(pipe_idx, buf);
            }
            // Regular file — fall through to VFS read.
            let fs = match morpheus_helix::vfs::global::fs_global_mut() {
                Some(fs) => fs,
                None => return ENOSYS,
            };
            let fd_table = SCHEDULER.current_fd_table_mut();
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
            return match morpheus_helix::vfs::vfs_read(
                &mut fs.device,
                &fs.mount_table,
                fd_table,
                fd as usize,
                buf,
            ) {
                Ok(n) => n as u64,
                Err(e) => helix_err_to_errno(e),
            };
        }
    }

    // fd_table has no entry for this fd — legacy fallback.
    match fd {
        0 => {
            // stdin — where does the data come from?
            //
            // Composited clients: per-process input buffer, populated by
            // the compositor via SYS_FORWARD_INPUT.  No pipes, no global
            // ring buffer, no hoping some intermediary remembered to forward
            // your keystrokes.
            //
            // Everyone else: global keyboard ring buffer (stdin), populated
            // by the PS/2 keyboard ISR.
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);

            if is_composited_client() {
                // Read from per-process input buffer.
                loop {
                    let proc = SCHEDULER.current_process_mut();
                    let mut n = 0usize;
                    while n < buf.len() && proc.input_tail != proc.input_head {
                        buf[n] = proc.input_buf[proc.input_tail as usize];
                        proc.input_tail = proc.input_tail.wrapping_add(1);
                        n += 1;
                    }
                    if n > 0 {
                        return n as u64;
                    }
                    // No data — check for pending signals before blocking.
                    if !proc.pending_signals.is_empty() {
                        return 0;
                    }
                    // Park until the compositor sends us something.
                    proc.state = crate::process::ProcessState::Blocked(
                        crate::process::BlockReason::InputRead,
                    );
                    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
                }
            }

            // Non-composited: global stdin ring buffer.
            loop {
                let n = crate::stdin::read(buf);
                if n > 0 {
                    return n as u64;
                }
                {
                    let proc = SCHEDULER.current_process_mut();
                    if !proc.pending_signals.is_empty() {
                        return 0;
                    }
                }
                {
                    let proc = SCHEDULER.current_process_mut();
                    proc.state = crate::process::ProcessState::Blocked(
                        crate::process::BlockReason::StdinRead,
                    );
                }
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
        }
        _ => EBADF,
    }
}

// SYS_YIELD — voluntary context switch

/// Yield. STI+HLT is atomic on x86-64 — no surprise interrupts.
pub unsafe fn sys_yield() -> u64 {
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    0
}

// SYS_GETPID

pub unsafe fn sys_getpid() -> u64 {
    SCHEDULER.current_pid() as u64
}

// SYS_KILL — send a signal to a process

/// `SYS_KILL(pid: u32, signal: u8)` — send signal to process.
pub unsafe fn sys_kill(pid: u64, signum: u64) -> u64 {
    let sig = match Signal::from_u8(signum as u8) {
        Some(s) => s,
        None => return u64::MAX - 22, // -EINVAL
    };
    match SCHEDULER.send_signal(pid as u32, sig) {
        Ok(_) => 0,
        Err(_) => u64::MAX - 3, // -ESRCH
    }
}

// SYS_WAIT — wait for a child process to exit

/// `SYS_WAIT(pid)` — block until child `pid` exits, then return its exit code.
///
/// If the child is already a Zombie, reaps immediately.
/// If `pid` is not a child of the caller, returns -ESRCH.
pub unsafe fn sys_wait(pid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(pid as u32)
}

// SYS_SLEEP — sleep for N milliseconds

/// `SYS_SLEEP(millis)` — suspend the calling process for at least `millis` ms.
///
/// Computes a TSC deadline and blocks with `BlockReason::Sleep(deadline)`.
/// The scheduler unblocks the process once the deadline has passed.
pub unsafe fn sys_sleep(millis: u64) -> u64 {
    if millis == 0 {
        return 0;
    }
    let tsc_freq = crate::process::scheduler::tsc_frequency();
    if tsc_freq == 0 {
        // TSC not calibrated — cannot compute deadline; return success anyway.
        return 0;
    }
    let ticks_per_ms = tsc_freq / 1000;
    let deadline = crate::cpu::tsc::read_tsc().saturating_add(millis.saturating_mul(ticks_per_ms));
    crate::process::scheduler::block_sleep(deadline)
}
