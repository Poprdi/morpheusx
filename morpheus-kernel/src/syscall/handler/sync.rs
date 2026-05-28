// Futex + thread syscalls + sigreturn + mouse_read.

use super::common::*;
use super::fb::is_composited_client;
use crate::hal;
use crate::mouse;
use crate::process;
use crate::process::{BlockReason, ProcessState};
use crate::schedular::SCHEDULER;
use morpheus_hal_api::Pml4Handle;

const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;

/// op=WAIT: block if `*addr == val` (EAGAIN otherwise); timeout=0 means forever.
/// op=WAKE: wake up to `val` waiters; `val=0` → 1.
pub unsafe fn sys_futex(addr: u64, op: u64, val: u64, timeout_ms: u64) -> u64 {
    if addr == 0 || addr & 3 != 0 || addr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if !validate_user_buf(addr, 4) {
        return EFAULT;
    }

    match op {
        FUTEX_WAIT => {
            let word_ptr = addr as *const u32;
            let current = core::ptr::read_volatile(word_ptr);

            if current != val as u32 {
                return u64::MAX - 11; // EAGAIN
            }

            {
                let proc = SCHEDULER.current_process_mut();
                crate::schedular::mark_futex_waiter(proc.pid, addr);
                proc.state = ProcessState::Blocked(BlockReason::FutexWait(addr));
                if timeout_ms > 0 {
                    let tsc_freq = crate::schedular::tsc_frequency();
                    if tsc_freq > 0 {
                        let ticks_per_ms = tsc_freq / 1000;
                        let deadline = hal()
                            .timer()
                            .read_tsc()
                            .saturating_add(timeout_ms.saturating_mul(ticks_per_ms));
                        proc.futex_deadline = deadline;
                        crate::schedular::inc_timed_block_count();
                        loop {
                            let current_earliest = crate::schedular::get_earliest_deadline();
                            if deadline < current_earliest {
                                if crate::schedular::try_set_earliest_deadline(
                                    current_earliest,
                                    deadline,
                                ) {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            hal().cpu().halt_wait_irq();
            0
        },
        FUTEX_WAKE => {
            let count = if val == 0 { 1 } else { val as u32 };
            crate::schedular::wake_futex_waiters(addr, count) as u64
        },
        _ => EINVAL,
    }
}

/// Thread shares caller's CR3. Caller pre-allocates `stack_top` via SYS_MMAP.
pub unsafe fn sys_thread_create(entry: u64, stack_top: u64, arg: u64) -> u64 {
    if entry == 0 || stack_top == 0 {
        return EINVAL;
    }
    if entry >= USER_ADDR_LIMIT || stack_top >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if stack_top & 0xF != 0 {
        return EINVAL; // x86-64 ABI: 16-byte stack alignment
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

    match crate::schedular::spawn_user_thread(entry, stack_top, arg) {
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
    let (dx, dy, buttons) = mouse::drain();
    let _ = process::ProcessState::Ready; // touch to keep import used
    let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
    let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
    (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32)
}
