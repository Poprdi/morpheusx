// Futex + thread syscalls + sigreturn + mouse_read.

use super::common::*;
use super::fb::is_composited_client;
use crate::hal;
use crate::mouse;
use crate::process;
use crate::process::{BlockReason, ProcessState};
use crate::schedular::{PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER};
use morpheus_hal_api::Pml4Handle;

use morpheus_foundation::errno::ETIMEDOUT;
use morpheus_foundation::flags::{
    FUTEX_WAIT, FUTEX_WAIT_BITSET, FUTEX_WAKE, FUTEX_WAKE_BITSET, THREAD_DETACHED,
};
use morpheus_foundation::types::Timespec;

/// Convert a RELATIVE `Timespec` (vs CLOCK_MONOTONIC) at `ts_ptr` to an absolute
/// TSC deadline. `Ok(0)` = block forever (NULL ptr or TSC uncalibrated);
/// `Err(EINVAL/EFAULT)` on a bad spec/pointer.
unsafe fn futex_deadline_from_timespec(ts_ptr: u64) -> Result<u64, u64> {
    if ts_ptr == 0 {
        return Ok(0);
    }
    if ts_ptr & 7 != 0 || !validate_user_buf(ts_ptr, core::mem::size_of::<Timespec>() as u64) {
        return Err(EFAULT);
    }
    let ts = core::ptr::read(ts_ptr as *const Timespec);
    if ts.tv_sec < 0 || ts.tv_nsec < 0 || ts.tv_nsec >= 1_000_000_000 {
        return Err(EINVAL);
    }
    let tsc_freq = crate::schedular::tsc_frequency();
    if tsc_freq == 0 {
        return Ok(0);
    }
    let total_ns = (ts.tv_sec as u128) * 1_000_000_000 + (ts.tv_nsec as u128);
    let ticks = (total_ns * tsc_freq as u128 / 1_000_000_000) as u64;
    Ok(hal().timer().read_tsc().saturating_add(ticks))
}

/// `arg4` (R10) is a `*const Timespec` RELATIVE timeout vs CLOCK_MONOTONIC, NULL =
/// forever. Returns 0 woken, `-ETIMEDOUT` expiry, `-EAGAIN` value-mismatch.
///
/// SMP lost-wakeup fix: the `*addr == val` compare AND the enqueue both run under
/// PROCESS_TABLE_LOCK — the same lock `wake_futex_waiters` takes. A cross-core
/// waker's userspace store happens-before its wake syscall, so either this thread
/// re-reads the new value under the lock (→ EAGAIN, no sleep) or it enqueues first
/// and the waker, taking the lock afterward, observes and wakes it.
pub unsafe fn sys_futex(addr: u64, op: u64, val: u64, timeout_ptr: u64) -> u64 {
    if addr == 0 || addr & 3 != 0 || addr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if !validate_user_buf(addr, 4) {
        return EFAULT;
    }

    match op {
        FUTEX_WAIT | FUTEX_WAIT_BITSET => {
            let deadline = match futex_deadline_from_timespec(timeout_ptr) {
                Ok(d) => d,
                Err(e) => return e,
            };

            let pid = SCHEDULER.current_pid();
            PROCESS_TABLE_LOCK.lock();

            // Re-read the word UNDER the lock — this is the linearization point.
            let current = core::ptr::read_volatile(addr as *const u32);
            if current != val as u32 {
                PROCESS_TABLE_LOCK.unlock();
                return EAGAIN;
            }

            if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
                proc.futex_timed_out = false;
                crate::schedular::mark_futex_waiter(pid, addr);
                proc.state = ProcessState::Blocked(BlockReason::FutexWait(addr));
                if deadline != 0 {
                    proc.futex_deadline = deadline;
                    crate::schedular::inc_timed_block_count();
                    loop {
                        let earliest = crate::schedular::get_earliest_deadline();
                        if deadline < earliest {
                            if crate::schedular::try_set_earliest_deadline(earliest, deadline) {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            PROCESS_TABLE_LOCK.unlock();

            hal().cpu().halt_wait_irq();

            let proc = SCHEDULER.current_process_mut();
            if proc.futex_timed_out {
                proc.futex_timed_out = false;
                ETIMEDOUT
            } else {
                0
            }
        },
        FUTEX_WAKE | FUTEX_WAKE_BITSET => {
            let count = if val == 0 { 1 } else { val as u32 };
            crate::schedular::wake_futex_waiters(addr, count) as u64
        },
        _ => EINVAL,
    }
}

/// Real thread sharing the caller's CR3. `tls_base` is set at creation (no
/// SET_THREAD_POINTER race); `ctid_ptr` is the CLONE_CHILD_CLEARTID slot the
/// kernel zeroes + FUTEX_WAKEs on exit; `flags & THREAD_DETACHED` auto-reaps.
/// Caller pre-allocates `stack_top` via SYS_MMAP.
pub unsafe fn sys_thread_create(
    entry: u64,
    stack_top: u64,
    arg: u64,
    tls_base: u64,
    ctid_ptr: u64,
    flags: u64,
) -> u64 {
    if entry == 0 || stack_top == 0 {
        return EINVAL;
    }
    if entry >= USER_ADDR_LIMIT || stack_top >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if stack_top & 0xF != 0 {
        return EINVAL; // x86-64 ABI: 16-byte stack alignment
    }
    if tls_base >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if ctid_ptr != 0 && (ctid_ptr & 3 != 0 || !validate_user_buf(ctid_ptr, 4)) {
        return EFAULT;
    }
    if flags & !THREAD_DETACHED != 0 {
        return EINVAL;
    }

    // Faulting on the first push would be unrecoverable.
    {
        let proc = SCHEDULER.current_process_mut();
        let cr3 = proc.cr3;

        let pml4 = Pml4Handle(cr3);
        let check_addr = stack_top - 8;
        let page_addr = check_addr & !0xFFF;
        if hal().paging().pml4_translate(pml4, page_addr).is_none() {
            return EFAULT;
        }
    }

    match crate::schedular::spawn_user_thread(entry, stack_top, arg, tls_base, ctid_ptr, flags) {
        Ok(tid) => tid as u64,
        Err(_) => ENOMEM,
    }
}

/// Threads are processes sharing CR3; uses normal exit path.
pub unsafe fn sys_thread_exit(code: u64) -> u64 {
    crate::schedular::exit_process(code as i32);
}

pub unsafe fn sys_thread_join(tid: u64) -> u64 {
    crate::schedular::wait_for_child(tid as u32)
}

/// Restore pre-signal context; EINVAL outside a handler.
pub unsafe fn sys_sigreturn() -> u64 {
    let proc = SCHEDULER.current_process_mut();
    if !proc.in_signal_handler {
        return EINVAL;
    }
    proc.context = proc.saved_signal_context;
    proc.fpu_state = proc.saved_signal_fpu;
    proc.in_signal_handler = false;
    // CpuContext is opaque; read RAX equivalent through the HAL accessor.
    hal().cpu().ctx_get_return(&proc.context)
}

/// Packed: dx i16 [15:0], dy i16 [31:16], buttons u8 [39:32].
pub unsafe fn sys_mouse_read() -> u64 {
    if is_composited_client() {
        let proc = SCHEDULER.current_process_mut();
        let dx = proc.mouse_dx;
        let dy = proc.mouse_dy;
        let buttons = proc.mouse_buttons;
        proc.mouse_dx = 0;
        proc.mouse_dy = 0;
        let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
        let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
        return (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32);
    }
    let (dx, dy, buttons, wheel) = mouse::drain();
    let _ = process::ProcessState::Ready; // touch to keep import used
    let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
    let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
    let wheel8 = (wheel.clamp(-128, 127) as i8) as u8;
    (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32) | ((wheel8 as u64) << 48)
}

/// SYS_THREAD_DETACH: `tid -> 0 | -errno`. Marks a sibling thread (same
/// thread-group leader) auto-reaping; a Zombie is freed by the next reaper sweep.
/// Any later join/wait on a detached thread returns `-ESRCH` (see `can_reap`).
/// Idempotent.
pub unsafe fn sys_thread_detach(tid: u64) -> u64 {
    if tid >= crate::process::MAX_PROCESSES as u64 {
        return ESRCH;
    }
    let caller_leader = SCHEDULER.current_memory_leader_pid();
    PROCESS_TABLE_LOCK.lock();
    let r = match PROCESS_TABLE.get_mut(tid as usize).and_then(|s| s.as_mut()) {
        Some(p) if !p.is_free() => {
            let same_group = p.is_thread() && p.thread_group_leader == caller_leader;
            if same_group || p.parent_pid == caller_leader {
                p.detached = true;
                0
            } else {
                ESRCH
            }
        },
        _ => ESRCH,
    };
    PROCESS_TABLE_LOCK.unlock();
    r
}

/// SYS_GETTID: `() -> tid`. The caller's per-thread id = its table slot. (getpid
/// returns the thread-group leader; both come from the one slot id allocator.)
pub unsafe fn sys_gettid() -> u64 {
    SCHEDULER.current_pid() as u64
}
