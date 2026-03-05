
// SYS_FUTEX (79) — userspace synchronization primitive

const FUTEX_WAIT: u64 = 0;
const FUTEX_WAKE: u64 = 1;

/// `SYS_FUTEX(addr, op, val, timeout_ms)` — futex wait/wake.
///
/// op=0 (WAIT): if `*addr == val`, block until woken or timeout_ms expires.
///              if `*addr != val`, return EAGAIN immediately.
///              timeout_ms=0 means wait forever.
/// op=1 (WAKE): wake up to `val` processes sleeping on `addr`.
///
/// addr must be 4-byte aligned and in user address space.
pub unsafe fn sys_futex(addr: u64, op: u64, val: u64, timeout_ms: u64) -> u64 {
    if addr == 0 || addr & 3 != 0 || addr >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    if !validate_user_buf(addr, 4) {
        return EFAULT;
    }

    match op {
        FUTEX_WAIT => {
            // Read the futex word atomically.
            let word_ptr = addr as *const u32;
            let current = core::ptr::read_volatile(word_ptr);

            // Spurious-safe: if someone already changed it, bail.
            if current != val as u32 {
                return u64::MAX - 11; // EAGAIN
            }

            // Block on this address.
            {
                let proc = SCHEDULER.current_process_mut();
                proc.state = crate::process::ProcessState::Blocked(
                    crate::process::BlockReason::FutexWait(addr),
                );
                // Set timeout deadline if requested.
                if timeout_ms > 0 {
                    let tsc_freq = crate::process::scheduler::tsc_frequency();
                    if tsc_freq > 0 {
                        let ticks_per_ms = tsc_freq / 1000;
                        let deadline = crate::cpu::tsc::read_tsc()
                            .saturating_add(timeout_ms.saturating_mul(ticks_per_ms));
                        proc.futex_deadline = deadline;
                        crate::process::scheduler::inc_timed_block_count();
                    }
                }
            }
            core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            // Check if we timed out (state was set back to Ready by the timer ISR).
            0
        }
        FUTEX_WAKE => {
            let count = if val == 0 { 1 } else { val as u32 };
            crate::process::scheduler::wake_futex_waiters(addr, count) as u64
        }
        _ => EINVAL,
    }
}

// SYS_THREAD_CREATE (80) — spawn a thread in the caller's address space

/// `SYS_THREAD_CREATE(entry, stack_top, arg) → tid`
///
/// Creates a new thread sharing the caller's page tables.  The thread
/// starts at `entry` with `rdi = arg` and `rsp = stack_top`.  Caller
/// must allocate the stack (via SYS_MMAP) before calling this.
pub unsafe fn sys_thread_create(entry: u64, stack_top: u64, arg: u64) -> u64 {
    if entry == 0 || stack_top == 0 {
        return EINVAL;
    }
    if entry >= USER_ADDR_LIMIT || stack_top >= USER_ADDR_LIMIT {
        return EINVAL;
    }
    // Stack must be 16-byte aligned (x86-64 ABI).
    if stack_top & 0xF != 0 {
        return EINVAL;
    }

    // Verify the first stack push target is mapped.
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

// SYS_THREAD_EXIT (81) — terminate the calling thread

/// `SYS_THREAD_EXIT(code)` — exits the current thread.
///
/// Same as SYS_EXIT under the hood — the scheduler handles thread vs
/// process distinction via thread_group_leader.
pub unsafe fn sys_thread_exit(code: u64) -> u64 {
    crate::process::scheduler::exit_process(code as i32);
}

// SYS_THREAD_JOIN (82) — wait for a thread to finish

/// `SYS_THREAD_JOIN(tid) → exit_code`
///
/// Blocks until the thread with `tid` exits.  Reuses the wait-for-child
/// mechanism since threads are just processes with shared CR3.
pub unsafe fn sys_thread_join(tid: u64) -> u64 {
    crate::process::scheduler::wait_for_child(tid as u32)
}

// SYS_SIGRETURN (83) — restore context after signal handler

/// `SYS_SIGRETURN() → 0`
///
/// Restores the saved pre-signal context.  Must be called by user signal
/// handlers when they are done.  If called outside a signal handler, returns
/// -EINVAL.
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

// SYS_MOUSE_READ (84) — read accumulated relative mouse state

/// `SYS_MOUSE_READ() → packed(dx:i16 | dy:i16 | buttons:u8)`
///
/// Returns accumulated relative motion since last call.
/// Bits [15:0] = dx (i16), [31:16] = dy (i16), [39:32] = buttons.
///
/// When a compositor is active:
///   - Compositor → reads the real hardware mouse accumulator.
///   - Other processes → reads their per-process mouse accumulator
///     (populated by the compositor via SYS_MOUSE_FORWARD).
pub unsafe fn sys_mouse_read() -> u64 {
    if is_composited_client() {
        let proc = SCHEDULER.current_process_mut();
        let dx = proc.mouse_dx;
        let dy = proc.mouse_dy;
        let buttons = proc.mouse_buttons;
        proc.mouse_dx = 0;
        proc.mouse_dy = 0;
        if dx != 0 || dy != 0 || buttons != 0 {
            use crate::serial::{put_hex32, puts};
            puts("[DBG] mouse_read client pid=");
            put_hex32(SCHEDULER.current_pid());
            puts(" dx=");
            put_hex32(dx as u32);
            puts(" dy=");
            put_hex32(dy as u32);
            puts(" btn=");
            put_hex32(buttons as u32);
            puts("\n");
        }
        let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
        let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
        return (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32);
    }
    let (dx, dy, buttons) = crate::mouse::drain();
    if dx != 0 || dy != 0 || buttons != 0 {
        use crate::serial::{put_hex32, puts};
        puts("[DBG] mouse_read hw pid=");
        put_hex32(SCHEDULER.current_pid());
        puts(" dx=");
        put_hex32(dx as u32);
        puts(" dy=");
        put_hex32(dy as u32);
        puts(" btn=");
        put_hex32(buttons as u32);
        puts("\n");
    }
    let dx16 = (dx.clamp(-32768, 32767) as i16) as u16;
    let dy16 = (dy.clamp(-32768, 32767) as i16) as u16;
    (dx16 as u64) | ((dy16 as u64) << 16) | ((buttons as u64) << 32)
}
