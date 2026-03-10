use super::lifecycle::terminate_process_inner;
use super::state::{
    clear_futex_waiter, clear_waiter_all, cleanup_stale_waiters,
    set_percpu_fpu_ptr, set_percpu_next_cr3, set_this_core_pid, this_core_index, this_core_pid,
    IDLE_TSC_TOTAL, KERNEL_CR3, KERNEL_HLT_ENTRY_TSC, KERNEL_LAST_WAS_IDLE, KERNEL_SKIP_STREAK,
    MAX_KERNEL_SKIP, PROCESS_TABLE, PROCESS_TABLE_LOCK, TICK_COUNT,
};
use crate::cpu::gdt::{KERNEL_CS, KERNEL_DS};
use crate::process::{BlockReason, CpuContext, ProcessState, MAX_PROCESSES};
use crate::process::signals::SignalAction;
use crate::cpu::per_cpu::MAX_CPUS;
use crate::serial::{put_hex32, puts};
use core::sync::atomic::Ordering;

const STARVATION_FORCE_TICKS: u32 = 64;
const STALE_WAITER_CLEANUP_INTERVAL: u32 = 1024; // low-cost cleanup every ~1k ticks

static mut AP_IDLE_CTX: [CpuContext; MAX_CPUS] = [const { CpuContext::empty() }; MAX_CPUS];

#[inline(never)]
unsafe fn ap_idle_hlt_loop() -> ! {
    loop {
        core::arch::asm!("sti; hlt; cli", options(nostack, nomem));
    }
}

unsafe fn ap_idle_context(core_idx: u32) -> &'static CpuContext {
    let ctx = &mut AP_IDLE_CTX[core_idx as usize];
    let pcpu = crate::cpu::per_cpu::current();
    ctx.rip = ap_idle_hlt_loop as u64;
    ctx.rsp = pcpu.boot_kernel_rsp;
    ctx.cs = KERNEL_CS as u64;
    ctx.ss = KERNEL_DS as u64;
    ctx.rflags = 0x202;
    ctx.rax = 0;
    ctx.rbx = 0;
    ctx.rcx = 0;
    ctx.rdx = 0;
    ctx.rsi = 0;
    ctx.rdi = 0;
    ctx.rbp = 0;
    ctx.r8 = 0;
    ctx.r9 = 0;
    ctx.r10 = 0;
    ctx.r11 = 0;
    ctx.r12 = 0;
    ctx.r13 = 0;
    ctx.r14 = 0;
    ctx.r15 = 0;
    ctx
}

#[no_mangle]
pub unsafe extern "C" fn scheduler_tick(current_ctx: &CpuContext) -> &'static CpuContext {
    let tick = TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    let now_tsc = crate::cpu::tsc::read_tsc();
    let core_idx = this_core_index();

    if core_idx == 0 {
        crate::syscall::handler::fb_present_tick();
        crate::ps2_mouse::poll();
    }

    PROCESS_TABLE_LOCK.lock();

    // proactive stale waiter cleanup every 1k ticks. prevents bit accumulation on long uptime.
    if core_idx == 0 && tick % STALE_WAITER_CLEANUP_INTERVAL == 0 {
        cleanup_stale_waiters();
    }

    let cur_pid = this_core_pid() as usize;

    if cur_pid == 0 && core_idx != 0 {
        deliver_pending_signals(0);
        wake_expired_sleepers();

        let next_pid = pick_next(0, true, core_idx);
        if next_pid == 0 {
            set_percpu_fpu_ptr(0);
            set_percpu_next_cr3(KERNEL_CR3);
            let pcpu = crate::cpu::per_cpu::current();
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
            pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
            PROCESS_TABLE_LOCK.unlock();
            return ap_idle_context(core_idx);
        }

        set_this_core_pid(next_pid as u32);
        let result = if let Some(Some(next)) = PROCESS_TABLE.get_mut(next_pid) {
            next.state = ProcessState::Running;
            next.run_start_tsc = now_tsc;
            next.running_on = core_idx;

            if next.kernel_stack_top != 0 {
                crate::cpu::gdt::set_kernel_stack_for_core(core_idx, next.kernel_stack_top);
                let pcpu = crate::cpu::per_cpu::current();
                pcpu.kernel_syscall_rsp = next.kernel_stack_top;
            }
            if crate::memory::is_valid_cr3(next.cr3) {
                set_percpu_next_cr3(next.cr3);
            }
            let fpu_ptr = &mut next.fpu_state as *mut crate::process::context::FpuState as u64;
            set_percpu_fpu_ptr(fpu_ptr);
            &next.context
        } else {
            set_this_core_pid(0);
            set_percpu_fpu_ptr(0);
            set_percpu_next_cr3(KERNEL_CR3);
            let pcpu = crate::cpu::per_cpu::current();
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
            pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
            PROCESS_TABLE_LOCK.unlock();
            return ap_idle_context(core_idx);
        };

        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    let hlt_entry = KERNEL_HLT_ENTRY_TSC.swap(0, Ordering::Relaxed);
    let kernel_was_idle = cur_pid == 0 && hlt_entry != 0 && core_idx == 0;

    if cur_pid == 0 && core_idx == 0 {
        KERNEL_LAST_WAS_IDLE.store(kernel_was_idle, Ordering::Relaxed);
    }

    if let Some(Some(cur)) = PROCESS_TABLE.get_mut(cur_pid) {
        cur.context = *current_ctx;

        // user-mode SS must keep RPL=3.
        if cur.context.cs & 3 == 3 {
            cur.context.ss |= 3;
        }

        cur.cpu_ticks += 1;

        if kernel_was_idle {
            let active_tsc = hlt_entry.saturating_sub(cur.run_start_tsc);
            let idle_tsc_q = now_tsc.saturating_sub(hlt_entry);
            cur.cpu_tsc = cur.cpu_tsc.saturating_add(active_tsc);
            IDLE_TSC_TOTAL.fetch_add(idle_tsc_q, Ordering::Relaxed);
        } else {
            cur.cpu_tsc = cur
                .cpu_tsc
                .saturating_add(now_tsc.saturating_sub(cur.run_start_tsc));
        }

        if cur.state == ProcessState::Running {
            cur.state = ProcessState::Ready;
        }
        cur.running_on = u32::MAX;
    }

    deliver_pending_signals(cur_pid as u32);
    wake_expired_sleepers();

    let compositor_active = crate::syscall::handler::compositor_active();
    let skip_kernel = if compositor_active || core_idx != 0 {
        core_idx != 0
    } else {
        KERNEL_LAST_WAS_IDLE.load(Ordering::Relaxed)
    };
    let next_pid = pick_next(cur_pid, skip_kernel, core_idx);

    if next_pid == 0 && core_idx != 0 {
        set_this_core_pid(0);
        set_percpu_fpu_ptr(0);
        set_percpu_next_cr3(KERNEL_CR3);
        let pcpu = crate::cpu::per_cpu::current();
        crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
        pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
        PROCESS_TABLE_LOCK.unlock();
        return ap_idle_context(core_idx);
    }

    set_this_core_pid(next_pid as u32);

    let result = if let Some(Some(next)) = PROCESS_TABLE.get_mut(next_pid) {
        next.state = ProcessState::Running;
        next.run_start_tsc = now_tsc;
        next.running_on = core_idx;

        if next.kernel_stack_top != 0 {
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, next.kernel_stack_top);
            if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
                let pcpu = crate::cpu::per_cpu::current();
                pcpu.kernel_syscall_rsp = next.kernel_stack_top;
            }
        }

        if crate::memory::is_valid_cr3(next.cr3) {
            set_percpu_next_cr3(next.cr3);
        }

        let fpu_ptr = &mut next.fpu_state as *mut crate::process::context::FpuState as u64;
        set_percpu_fpu_ptr(fpu_ptr);

        &next.context
    } else {
        if core_idx != 0 {
            set_this_core_pid(0);
            set_percpu_fpu_ptr(0);
            set_percpu_next_cr3(KERNEL_CR3);
            let pcpu = crate::cpu::per_cpu::current();
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
            pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
            PROCESS_TABLE_LOCK.unlock();
            return ap_idle_context(core_idx);
        }
        set_this_core_pid(0);
        set_percpu_fpu_ptr(0);
        set_percpu_next_cr3(KERNEL_CR3);
        if let Some(Some(cur)) = PROCESS_TABLE.get(cur_pid) {
            &cur.context
        } else {
            core::mem::transmute::<&CpuContext, &'static CpuContext>(current_ctx)
        }
    };

    PROCESS_TABLE_LOCK.unlock();

    result
}

pub(super) unsafe fn wake_expired_sleepers() {
    let timed_count = super::state::TIMED_BLOCK_COUNT.load(Ordering::Relaxed);
    if timed_count == 0 {
        return;
    }
    let now = crate::cpu::tsc::read_tsc();
    let earliest = super::state::EARLIEST_DEADLINE.load(Ordering::Relaxed);
    if now < earliest {
        return;
    }

    let mut found_any = false;
    let mut new_earliest = u64::MAX;

    for proc in PROCESS_TABLE.iter_mut().flatten() {
        match proc.state {
            ProcessState::Blocked(BlockReason::Sleep(deadline)) => {
                if now >= deadline {
                    proc.state = ProcessState::Ready;
                    super::state::TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                } else {
                    new_earliest = new_earliest.min(deadline);
                    found_any = true;
                }
            }
            ProcessState::Blocked(BlockReason::FutexWait(_)) => {
                if proc.futex_deadline != 0 && now >= proc.futex_deadline {
                    if let ProcessState::Blocked(BlockReason::FutexWait(addr)) = proc.state {
                        clear_futex_waiter(proc.pid, addr);
                    }
                    proc.state = ProcessState::Ready;
                    proc.futex_deadline = 0;
                    super::state::TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                } else if proc.futex_deadline != 0 {
                    new_earliest = new_earliest.min(proc.futex_deadline);
                    found_any = true;
                }
            }
            _ => {}
        }
    }

    // update earliest deadline for next wake check. helps avoid redundant O(N) scans.
    if found_any {
        super::state::EARLIEST_DEADLINE.store(new_earliest, Ordering::Relaxed);
    } else {
        super::state::EARLIEST_DEADLINE.store(u64::MAX, Ordering::Relaxed);
    }
}

unsafe fn pick_next(current: usize, skip_kernel: bool, core_idx: u32) -> usize {
    let n = MAX_PROCESSES;
    let is_bsp = core_idx == 0;

    #[inline(always)]
    fn priority_weight(priority: u8) -> u8 {
        // 0 = highest priority. map to 8..1 weight buckets.
        1 + ((255u16.saturating_sub(priority as u16) >> 5) as u8)
    }

    let streak = KERNEL_SKIP_STREAK.load(Ordering::Relaxed);
    let skip_kernel = if is_bsp && skip_kernel && streak >= MAX_KERNEL_SKIP {
        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
        false
    } else {
        skip_kernel
    };

    // BSP ages ready tasks once per global tick to keep starvation bounded.
    if is_bsp {
        for proc in PROCESS_TABLE.iter_mut().flatten() {
            if proc.state == ProcessState::Ready && proc.running_on == u32::MAX {
                proc.sched_wait_ticks = proc.sched_wait_ticks.saturating_add(1);
            } else if proc.state != ProcessState::Running {
                proc.sched_wait_ticks = 0;
                proc.sched_budget_left = 0;
            }
        }
    }

    // Hard starvation bound: any long-waiting ready task preempts RR order.
    let mut forced_starving: Option<usize> = None;
    for delta in 1..=n {
        let candidate = (current + delta) % n;
        if candidate == 0 && (!is_bsp || skip_kernel) {
            continue;
        }
        if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
            if p.running_on == u32::MAX
                && p.state == ProcessState::Ready
                && p.sched_wait_ticks >= STARVATION_FORCE_TICKS
            {
                forced_starving = Some(candidate);
                break;
            }
        }
    }

    if let Some(candidate) = forced_starving {
        if let Some(Some(p)) = PROCESS_TABLE.get_mut(candidate) {
            if p.sched_budget_left == 0 {
                p.sched_budget_left = priority_weight(p.priority);
            }
            p.sched_budget_left = p.sched_budget_left.saturating_sub(1);
            p.sched_wait_ticks = 0;
        }
        if is_bsp {
            if candidate == 0 {
                KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
            } else if skip_kernel {
                KERNEL_SKIP_STREAK.fetch_add(1, Ordering::Relaxed);
            }
        }
        return candidate;
    }

    // Weighted RR pass: pick first ready task with remaining budget.
    for delta in 1..=n {
        let candidate = (current + delta) % n;
        if candidate == 0 && (!is_bsp || skip_kernel) {
            continue;
        }
        if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
            if p.running_on != u32::MAX {
                continue;
            }
            if p.state == ProcessState::Ready && p.sched_budget_left > 0 {
                if let Some(Some(pm)) = PROCESS_TABLE.get_mut(candidate) {
                    pm.sched_budget_left = pm.sched_budget_left.saturating_sub(1);
                    pm.sched_wait_ticks = 0;
                }
                if is_bsp {
                    if candidate == 0 {
                        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
                    } else if skip_kernel {
                        KERNEL_SKIP_STREAK.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return candidate;
            }
        }
    }

    // Epoch rollover: no one had budget, refill ready tasks by priority weight.
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if proc.state == ProcessState::Ready && proc.running_on == u32::MAX {
            proc.sched_budget_left = priority_weight(proc.priority);
        }
    }

    for delta in 1..=n {
        let candidate = (current + delta) % n;
        if candidate == 0 && (!is_bsp || skip_kernel) {
            continue;
        }
        if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
            if p.running_on != u32::MAX {
                continue;
            }
            if p.state == ProcessState::Ready && p.sched_budget_left > 0 {
                if let Some(Some(pm)) = PROCESS_TABLE.get_mut(candidate) {
                    pm.sched_budget_left = pm.sched_budget_left.saturating_sub(1);
                    pm.sched_wait_ticks = 0;
                }
                if is_bsp {
                    if candidate == 0 {
                        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
                    } else if skip_kernel {
                        KERNEL_SKIP_STREAK.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return candidate;
            }
        }
    }

    if is_bsp {
        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
        for delta in 1..=n {
            let candidate = (current + delta) % n;
            if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
                if p.running_on != u32::MAX {
                    continue;
                }
                if p.state == ProcessState::Ready {
                    if let Some(Some(pm)) = PROCESS_TABLE.get_mut(candidate) {
                        if pm.sched_budget_left == 0 {
                            pm.sched_budget_left = priority_weight(pm.priority);
                        }
                        pm.sched_budget_left = pm.sched_budget_left.saturating_sub(1);
                        pm.sched_wait_ticks = 0;
                    }
                    return candidate;
                }
            }
        }
    }

    if let Some(Some(p)) = PROCESS_TABLE.get(current) {
        if p.state.is_runnable() && !(current == 0 && !is_bsp) {
            if let Some(Some(pm)) = PROCESS_TABLE.get_mut(current) {
                if pm.sched_budget_left == 0 {
                    pm.sched_budget_left = priority_weight(pm.priority);
                }
                pm.sched_budget_left = pm.sched_budget_left.saturating_sub(1);
                pm.sched_wait_ticks = 0;
            }
            return current;
        }
    }
    0
}

pub(super) unsafe fn deliver_pending_signals(pid: u32) {
    let proc = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
        Some(p) => p,
        None => return,
    };

    if proc.in_signal_handler {
        return;
    }

    while let Some(sig) = proc.pending_signals.take_next() {
        let handler = proc
            .signal_handlers
            .get(sig as u8 as usize)
            .copied()
            .unwrap_or(0);
        if handler == 1 {
            continue;
        }
        if handler > 1 {
            proc.saved_signal_context = proc.context;
            proc.saved_signal_fpu = proc.fpu_state;
            proc.in_signal_handler = true;

            let aligned_rsp = (proc.context.rsp & !0xF) - 8;
            proc.context.rip = handler;
            proc.context.rdi = sig as u8 as u64;
            proc.context.rsp = aligned_rsp;

            return;
        }
        match sig.default_action() {
            SignalAction::Terminate => {
                puts("[SCHED] signal -> PID ");
                put_hex32(pid);
                puts(" terminated\n");
                terminate_process_inner(proc, -(sig as u8 as i32));
                return;
            }
            SignalAction::Stop => {
                clear_waiter_all(pid);
                proc.state = ProcessState::Blocked(BlockReason::Io);
                return;
            }
            SignalAction::Continue => {
                if let ProcessState::Blocked(_) = proc.state {
                    clear_waiter_all(pid);
                    proc.state = ProcessState::Ready;
                }
            }
            SignalAction::Ignore => {}
        }
    }
}
