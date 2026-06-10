// Core syscalls: write/read/yield/exit/getpid/kill/wait/sleep/system_control.
// All syscalls: u64 args, u64 return. High bits = errno (u64::MAX - n).

use super::common::*;
use super::fb::is_composited_client;
use crate::hal;
use crate::process::signals::Signal;
use crate::process::{BlockReason, ProcessState};
use crate::schedular::{exit_process, SCHEDULER};
use crate::serial::puts;
use morpheus_helix::types::open_flags::{O_PIPE_READ, O_PIPE_WRITE};

use morpheus_foundation::flags::{
    SYSCTL_REBOOT_FORCE, SYSCTL_REBOOT_GRACEFUL, SYSCTL_SHUTDOWN_FORCE, SYSCTL_SHUTDOWN_GRACEFUL,
    SYSCTL_SHUTDOWN_PANIC,
};

static SYSTEM_CONTROL_IN_PROGRESS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// `SYS_EXIT(code)` — never returns.
pub unsafe fn sys_exit(code: u64) -> u64 {
    exit_process(code as i32);
}

/// fd 0/1/2: dup2'd target (pipe/file) if set; else fd 1/2 go to serial,
/// fd 0 returns EBADF. fd >= 3: pipe or VFS.
pub unsafe fn sys_write(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }

    // Redirected fds (via dup2) take precedence over stdio defaults.
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
                crate::schedular::wake_pipe_readers(pipe_idx);
                return n as u64;
            }
            let mut _vfs_guard = match vfs_lock() {
                Some(g) => g,
                None => return ENOSYS,
            };
            let fs = &mut *_vfs_guard.fs;
            let fd_table = SCHEDULER.current_fd_table_mut();
            let data = core::slice::from_raw_parts(ptr as *const u8, len as usize);
            let ts = hal().timer().read_tsc();
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

    // Default stdio routing.
    match fd {
        1 | 2 => {
            let bytes = core::slice::from_raw_parts(ptr as *const u8, len as usize);
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
        },
        _ => EBADF,
    }
}

/// fd 0/1/2 honor dup2 redirects; else fd 0 reads stdin ring, fd 1/2 EBADF.
pub unsafe fn sys_read(fd: u64, ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }

    {
        let fd_table = SCHEDULER.current_fd_table_mut();
        if let Ok(desc) = fd_table.get(fd as usize) {
            if desc.flags & O_PIPE_READ != 0 {
                let pipe_idx = desc.mount_idx;
                let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
                return super::ipc::sys_pipe_read_blocking(pipe_idx, buf);
            }
            let mut _vfs_guard = match vfs_lock() {
                Some(g) => g,
                None => return ENOSYS,
            };
            let fs = &mut *_vfs_guard.fs;
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

    match fd {
        0 => {
            // Composited clients: per-process input buf fed by SYS_FORWARD_INPUT.
            // Others: global stdin ring fed by the PS/2 ISR.
            let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);

            let nonblock = crate::process::stdin_nonblock(SCHEDULER.current_process_mut().pid);

            if is_composited_client() {
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
                    if nonblock {
                        return EAGAIN;
                    }
                    if !proc.pending_signals.is_empty() {
                        return 0;
                    }
                    crate::schedular::mark_input_waiter(proc.pid);
                    proc.state = ProcessState::Blocked(BlockReason::InputRead);
                    hal().cpu().halt_wait_irq();
                }
            }

            loop {
                let n = crate::stdin::read(buf);
                if n > 0 {
                    return n as u64;
                }
                if nonblock {
                    return EAGAIN;
                }
                {
                    let proc = SCHEDULER.current_process_mut();
                    if !proc.pending_signals.is_empty() {
                        return 0;
                    }
                }
                {
                    let proc = SCHEDULER.current_process_mut();
                    crate::schedular::mark_stdin_waiter(proc.pid);
                    proc.state = ProcessState::Blocked(BlockReason::StdinRead);
                }
                hal().cpu().halt_wait_irq();
            }
        },
        _ => EBADF,
    }
}

/// Non-blocking drain of the kernel keyboard event ring into `buf`, as raw
/// PS/2 Set 1 bytes (break encoded as `|0x80`, `0xE0` as its own byte —
/// exactly what `morpheus-xhci`'s HID driver pushes). Returns the number of
/// bytes written (0 if the ring is empty).
///
/// Symmetric with [`super::sync::sys_mouse_read`]: input events are delivered
/// through a dedicated ring-draining syscall, not the stdin byte stream. The
/// compositor owns this; it then forwards decoded input to focused windows via
/// `SYS_FORWARD_INPUT`.
pub unsafe fn sys_keyboard_read(ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 || len > (1 << 20) {
        return EINVAL;
    }
    if !validate_user_buf(ptr, len) {
        return EFAULT;
    }
    let buf = core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize);
    let mut n = 0usize;
    while n < buf.len() {
        match crate::input::poll_keyboard() {
            Some(crate::input::InputEvent::Key(byte, _process)) => {
                buf[n] = byte;
                n += 1;
            },
            Some(_) => continue,
            None => break,
        }
    }
    n as u64
}

/// Atomic-IRQ-wait on x86 (`sti; hlt; cli`) routed through HAL.
pub unsafe fn sys_yield() -> u64 {
    hal().cpu().halt_wait_irq();
    0
}

pub unsafe fn sys_getpid() -> u64 {
    SCHEDULER.current_pid() as u64
}

pub unsafe fn sys_kill(pid: u64, signum: u64) -> u64 {
    let sig = match Signal::from_u8(signum as u8) {
        Some(s) => s,
        None => return EINVAL, // bad signal number
    };
    match SCHEDULER.send_signal(pid as u32, sig) {
        Ok(_) => 0,
        Err(_) => ESRCH,
    }
}

pub unsafe fn sys_wait(pid: u64) -> u64 {
    crate::schedular::wait_for_child(pid as u32)
}

pub unsafe fn sys_sleep(millis: u64) -> u64 {
    if millis == 0 {
        return 0;
    }
    let tsc_freq = crate::schedular::tsc_frequency();
    if tsc_freq == 0 {
        // TSC uncalibrated — best-effort no-op.
        return 0;
    }
    let ticks_per_ms = tsc_freq / 1000;
    let deadline = hal()
        .timer()
        .read_tsc()
        .saturating_add(millis.saturating_mul(ticks_per_ms));
    crate::schedular::block_sleep(deadline)
}

pub unsafe fn sys_system_control(mode: u64) -> u64 {
    // Single owner — racing reboot/shutdown callers get EBUSY.
    if SYSTEM_CONTROL_IN_PROGRESS
        .compare_exchange(
            false,
            true,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return EBUSY;
    }

    let owner_core = hal().smp().current_core_index();
    hal().smp().set_reboot_owner(owner_core);
    crate::shutdown::ensure_initialized();

    match mode {
        SYSCTL_REBOOT_FORCE => hard_reset_now(crate::shutdown::TransitionKind::RebootForce),
        SYSCTL_SHUTDOWN_FORCE => hard_reset_now(crate::shutdown::TransitionKind::ShutdownForce),
        SYSCTL_SHUTDOWN_PANIC => {
            // Take the crash-handler path: shows BSOD, then resets.
            hal().cpu().set_reset_on_crash(true);
            hal().cpu().crash_now()
        },
        SYSCTL_REBOOT_GRACEFUL => {
            graceful_reset_now(crate::shutdown::TransitionKind::RebootGraceful)
        },
        SYSCTL_SHUTDOWN_GRACEFUL => {
            graceful_reset_now(crate::shutdown::TransitionKind::ShutdownGraceful)
        },
        _ => {
            SYSTEM_CONTROL_IN_PROGRESS.store(false, core::sync::atomic::Ordering::Release);
            hal().smp().clear_reboot_owner();
            EINVAL
        },
    }
}

unsafe fn graceful_reset_now(kind: crate::shutdown::TransitionKind) -> ! {
    const MAX_SNAPSHOT: usize = 64;
    const DRAIN_ROUNDS: usize = 24;
    const DRAIN_BACKOFF_SPINS: usize = 200_000;

    let caller = SCHEDULER.current_pid();

    crate::serial::set_checkpoints_enabled(true);
    crate::serial::checkpoint("shutdown-prepare-begin");
    let prepare_ok = crate::shutdown::run_prepare_handlers(kind, 300);
    if prepare_ok {
        crate::serial::checkpoint("shutdown-prepare-complete");
    } else {
        crate::serial::checkpoint("shutdown-prepare-incomplete");
    }

    crate::serial::fb_puts("[INFO] [SHUTDOWN] draining processes\n");
    crate::serial::checkpoint("shutdown-drain-begin");

    // TERM half the rounds, then KILL survivors.
    for round in 0..DRAIN_ROUNDS {
        let mut procs = [crate::schedular::ProcessInfo::zeroed(); MAX_SNAPSHOT];
        let n = SCHEDULER.snapshot_processes(&mut procs);

        let mut alive_user = 0usize;
        for p in &procs[..n] {
            let pid = p.pid;
            if pid == 0
                || pid == caller
                || matches!(p.state, ProcessState::Terminated | ProcessState::Zombie)
            {
                continue;
            }
            alive_user += 1;
            let sig = if round < (DRAIN_ROUNDS / 2) {
                Signal::SIGTERM
            } else {
                Signal::SIGKILL
            };
            let _ = SCHEDULER.send_signal(pid, sig);
        }

        if alive_user == 0 {
            crate::serial::checkpoint("shutdown-drain-empty");
            break;
        }

        // No HLT — a masked/misrouted timer IRQ would deadlock the reset path.
        if round == (DRAIN_ROUNDS / 2) {
            crate::serial::checkpoint("shutdown-drain-escalate-sigkill");
        }
        for _ in 0..DRAIN_BACKOFF_SPINS {
            core::hint::spin_loop();
        }
    }

    crate::serial::fb_puts("[INFO] [SHUTDOWN] entering reset sequence\n");
    crate::serial::checkpoint("shutdown-reset-seq");

    // No VFS lock here — reset must always complete.
    hard_reset_now(kind)
}

unsafe fn hard_reset_now(kind: crate::shutdown::TransitionKind) -> ! {
    match kind {
        crate::shutdown::TransitionKind::RebootGraceful
        | crate::shutdown::TransitionKind::RebootForce => {
            crate::shutdown::run_restart_handlers(kind);
        },
        crate::shutdown::TransitionKind::ShutdownGraceful
        | crate::shutdown::TransitionKind::ShutdownForce => {
            crate::shutdown::run_poweroff_handlers(kind);
        },
    }
    hal().reset().reset_machine()
}
