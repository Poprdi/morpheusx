use super::state::{
    clear_waiter_all, sample_idle_tsc_total,
    sample_per_core_idle_tsc as state_sample_per_core_idle_tsc, set_percpu_fpu_ptr,
    set_this_core_pid, this_core_pid, KERNEL_CR3, KERNEL_HLT_ENTRY_TSC, LIVE_COUNT, PROCESS_TABLE,
    PROCESS_TABLE_LOCK, SCHEDULER_READY, TIMED_BLOCK_COUNT, TSC_FREQUENCY,
};
use crate::hal;
use crate::process::{
    BlockReason, CpuContext, Process, ProcessPolicyClass, ProcessPowerMode, ProcessState, Signal,
    MAX_PROCESSES, SCHED_CAP_DEFAULT,
};
use crate::sched_hooks;
use crate::serial::{put_hex32, puts};
use core::sync::atomic::Ordering;

pub unsafe fn set_tsc_frequency(freq: u64) {
    TSC_FREQUENCY = freq;
}

pub fn tsc_frequency() -> u64 {
    unsafe { TSC_FREQUENCY }
}

pub fn mark_kernel_hlt() {
    KERNEL_HLT_ENTRY_TSC.store(hal().timer().read_tsc(), Ordering::Relaxed);
}

pub fn idle_tsc_total() -> u64 {
    sample_idle_tsc_total(hal().timer().read_tsc())
}

pub fn sample_per_core_idle_tsc(out: &mut [u64]) -> usize {
    state_sample_per_core_idle_tsc(hal().timer().read_tsc(), out)
}

pub fn inc_timed_block_count() {
    TIMED_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Cancel a timed block early (readiness/futex wakeup before its deadline) so the
/// tick fast-path's count stays accurate.
pub fn dec_timed_block_count() {
    TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
}

pub(super) fn apply_default_scheduler_policy(proc: &mut Process, is_kernel: bool) {
    proc.importance_16 = 8;
    proc.power_mode = ProcessPowerMode::Balanced;
    proc.policy_class = ProcessPolicyClass::Throughput;
    proc.capability_bits = SCHED_CAP_DEFAULT;
    proc.affinity_mask = u64::MAX;
    proc.policy_flags = 0;
    proc.effective_weight_cache = 0;

    if is_kernel {
        proc.importance_16 = 16;
        proc.power_mode = ProcessPowerMode::Performance;
        proc.policy_class = ProcessPolicyClass::LatencyCritical;
    }
}

pub unsafe fn init_scheduler() {
    if SCHEDULER_READY {
        puts("[SCHED] already initialized\n");
        return;
    }

    let mut kernel_proc = Process::empty();
    kernel_proc.pid = 0;
    kernel_proc.set_name("kernel");
    kernel_proc.state = ProcessState::Running;
    kernel_proc.priority = 0;
    kernel_proc.running_on = 0;
    apply_default_scheduler_policy(&mut kernel_proc, true);

    let cr3 = hal().paging().current_cr3();
    kernel_proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;
    KERNEL_CR3 = kernel_proc.cr3;

    PROCESS_TABLE[0] = Some(kernel_proc);
    LIVE_COUNT.store(1, Ordering::SeqCst);
    set_this_core_pid(0);
    SCHEDULER_READY = true;

    if let Some(p) = PROCESS_TABLE[0].as_mut() {
        // PID 0 never goes through alloc_kernel_stack (it allocates no kernel
        // stack), so seed its FPU control words here (FCW=0x037F, MXCSR=0x1F80)
        // rather than leaving the zeroed state spawned procs avoid.
        hal().cpu().fpu_init(&mut p.fpu_state);
        let fpu_ptr = &mut p.fpu_state as *mut morpheus_hal_api::FpuState as u64;
        set_percpu_fpu_ptr(fpu_ptr);
    }
}

pub unsafe fn spawn_kernel_thread(
    name: &str,
    entry_fn: u64,
    priority: u8,
) -> Result<u32, &'static str> {
    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    PROCESS_TABLE_LOCK.lock();

    let slot_idx = (1..MAX_PROCESSES)
        .find(|&i| {
            PROCESS_TABLE[i]
                .as_ref()
                .map(|p| p.is_free())
                .unwrap_or(true)
        })
        .ok_or_else(|| {
            PROCESS_TABLE_LOCK.unlock();
            "process table full"
        })?;

    let pid = slot_idx as u32;

    let mut proc = Process::empty();
    proc.pid = pid;
    proc.set_name(name);
    proc.parent_pid = this_core_pid();
    proc.priority = priority;
    proc.state = ProcessState::Ready;
    apply_default_scheduler_policy(&mut proc, true);

    let cr3 = hal().paging().current_cr3();
    proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    if let Err(e) = proc.alloc_kernel_stack() {
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    {
        // CpuContext is opaque; HAL applies arch-side selectors (KERNEL_CS/DS).
        proc.context = CpuContext::zeroed();
        hal()
            .cpu()
            .ctx_init_kernel(&mut proc.context, entry_fn, proc.kernel_stack_top);
    }

    let _ = (pid, entry_fn);
    crate::serial::log_info("SCHED", 770, "kernel thread spawned");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    PROCESS_TABLE_LOCK.unlock();
    Ok(pid)
}

/// Canonical lower-half limit; mirrors `syscall::handler::common::USER_ADDR_LIMIT`.
const USER_ADDR_LIMIT: u64 = 0x0000_8000_0000_0000;

pub unsafe fn exit_process(code: i32) -> ! {
    let pid = this_core_pid();

    // CLONE_CHILD_CLEARTID: zero `ctid` and FUTEX_WAKE it BEFORE zombifying, while
    // this thread's CR3 is still loaded so the store lands in the shared address
    // space any joining sibling reads. Done outside the table lock (the wake takes
    // it itself); a joiner that observes 0 returns from join without racing the
    // reap. The slot itself is reaped later, off this (in-use) kernel stack.
    let ctid = PROCESS_TABLE
        .get(pid as usize)
        .and_then(|s| s.as_ref())
        .map(|p| p.ctid_ptr)
        .unwrap_or(0);
    if ctid != 0 && ctid & 3 == 0 && ctid < USER_ADDR_LIMIT {
        core::ptr::write_volatile(ctid as *mut u32, 0);
        crate::schedular::wake_futex_waiters(ctid, u32::MAX);
    }

    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        terminate_process_inner(proc, code, 0);
    }
    PROCESS_TABLE_LOCK.unlock();

    hal().cpu().enable_interrupts();
    loop {
        hal().cpu().halt_no_irq();
    }
}

/// `term_signal != 0` ⇒ the task was killed by that signal (drives WIFSIGNALED);
/// 0 ⇒ a normal exit with `code`.
pub(super) unsafe fn terminate_process_inner(proc: &mut Process, code: i32, term_signal: u8) {
    let child_pid = proc.pid;
    let parent_pid = proc.parent_pid;

    clear_waiter_all(child_pid);

    sched_hooks::release_fb_lock_if_holder(child_pid);

    // Release this task's pipe endpoints at exit (not at reap): a child that exits
    // must drop its writer immediately so a parent already blocked in a pipe read
    // observes EOF, even though the zombie slot is reaped later. File fds keep
    // their reap-time accounting; only pipe peers need prompt closure. Readers
    // parked on a now-writerless pipe are woken in the sweep below.
    {
        use morpheus_foundation::flags::open_flags::{O_PIPE_READ, O_PIPE_WRITE};
        let mut pipe_fds: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
        for (fd, desc) in proc.fd_table.iter() {
            if desc.flags & (O_PIPE_READ | O_PIPE_WRITE) == 0 {
                continue;
            }
            let idx = desc.mount_id as u8;
            if desc.flags & O_PIPE_READ != 0 {
                crate::pipe::pipe_close_reader(idx);
            }
            if desc.flags & O_PIPE_WRITE != 0 {
                crate::pipe::pipe_close_writer(idx);
            }
            pipe_fds.push(fd);
        }
        for fd in pipe_fds {
            proc.fd_table.free(fd);
        }
    }

    proc.state = ProcessState::Zombie;
    proc.exit_code = Some(code);
    proc.term_signal = term_signal;
    LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);

    puts("[SCHED] PID ");
    put_hex32(child_pid);
    puts(" exited code=");
    put_hex32(code as u32);
    puts("\n");

    // Wake every task parked in waitpid/thread-join for this child specifically
    // or for "any child" (id 0). The woken waiter re-validates reaper eligibility,
    // so this deliberately-broad sweep enables join-from-ANY-thread without the
    // terminate path needing to know the thread-group topology.
    for idx in 0..MAX_PROCESSES {
        if idx == child_pid as usize {
            continue;
        }
        if let Some(Some(p)) = PROCESS_TABLE.get_mut(idx) {
            if let ProcessState::Blocked(BlockReason::WaitChild(waited)) = p.state {
                if waited == child_pid || waited == 0 {
                    p.state = ProcessState::Ready;
                }
            }
            // A reader parked on a pipe whose last writer just closed must wake to
            // collect its EOF.
            if let ProcessState::Blocked(BlockReason::PipeRead(widx)) = p.state {
                if crate::pipe::pipe_writers(widx) == 0 {
                    p.state = ProcessState::Ready;
                }
            }
        }
    }
    if let Some(Some(parent)) = PROCESS_TABLE.get_mut(parent_pid as usize) {
        parent.pending_signals.raise(Signal::SIGCHLD);
    }
}
