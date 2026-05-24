
const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;

/// op=WAIT: block if `*addr == val` (EAGAIN otherwise); timeout=0 means forever.
/// op=WAKE: wake up to `val` waiters; `val=0` → 1.
/// `addr` must be 4-byte aligned and user.
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

            // Spurious-safe: someone changed it before we blocked.
            if current != val as u32 {
                return u64::MAX - 11; // EAGAIN
            }

            {
                let proc = SCHEDULER.current_process_mut();
                crate::process::scheduler::mark_futex_waiter(proc.pid, addr);
                proc.state = crate::process::ProcessState::Blocked(
                    crate::process::BlockReason::FutexWait(addr),
                );
                if timeout_ms > 0 {
                    let tsc_freq = crate::process::scheduler::tsc_frequency();
                    if tsc_freq > 0 {
                        let ticks_per_ms = tsc_freq / 1000;
                        let deadline = crate::cpu::tsc::read_tsc()
                            .saturating_add(timeout_ms.saturating_mul(ticks_per_ms));
                        proc.futex_deadline = deadline;
                        crate::process::scheduler::inc_timed_block_count();
                        // Race-free min update for the tick fast-path.
                        loop {
                            let current_earliest = crate::process::scheduler::get_earliest_deadline();
                            if deadline < current_earliest {
                                if crate::process::scheduler::try_set_earliest_deadline(current_earliest, deadline) {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            0
        }
        FUTEX_WAKE => {
            let count = if val == 0 { 1 } else { val as u32 };
            crate::process::scheduler::wake_futex_waiters(addr, count) as u64
        }
        _ => EINVAL,
    }
}

/// Thread shares caller's CR3. Caller pre-allocates `stack_top` via SYS_MMAP.
/// Entry frame: rip=entry, rdi=arg, rsp=stack_top.
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

        let ptm = crate::paging::table::PageTableManager { pml4_phys: cr3 };
        let check_addr = stack_top - 8;
        let page_addr = check_addr & !0xFFF;
        if ptm.translate(page_addr).is_none() {
            return EFAULT;
        }
    }

    match crate::process::spawn_user_thread(entry, stack_top, arg) {
        Ok(tid) => tid as u64,
        Err(_) => ENOMEM,
    }
}

/// Threads are processes sharing CR3; uses normal exit path.
pub unsafe fn sys_thread_exit(code: u64) -> u64 {
    crate::process::scheduler::exit_process(code as i32);
}

pub unsafe fn sys_thread_join(tid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(tid as u32)
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
    proc.context.rax
}

/// Packed: dx i16 [15:0], dy i16 [31:16], buttons u8 [39:32].
/// Composited clients see their per-process accumulator (fed by SYS_MOUSE_FORWARD).
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
    let (dx, dy, buttons) = crate::mouse::drain();
    let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
    let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
    (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32)
}
