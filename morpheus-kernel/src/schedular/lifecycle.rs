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

pub unsafe fn exit_process(code: i32) -> ! {
    let pid = this_core_pid();

    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        terminate_process_inner(proc, code);
    }
    PROCESS_TABLE_LOCK.unlock();

    hal().cpu().enable_interrupts();
    loop {
        hal().cpu().halt_no_irq();
    }
}

pub(super) unsafe fn terminate_process_inner(proc: &mut Process, code: i32) {
    let child_pid = proc.pid;
    let parent_pid = proc.parent_pid;

    clear_waiter_all(child_pid);

    sched_hooks::release_fb_lock_if_holder(child_pid);

    proc.state = ProcessState::Zombie;
    proc.exit_code = Some(code);
    LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);

    puts("[SCHED] PID ");
    put_hex32(child_pid);
    puts(" exited code=");
    put_hex32(code as u32);
    puts("\n");

    if let Some(Some(parent)) = PROCESS_TABLE.get_mut(parent_pid as usize) {
        if let ProcessState::Blocked(BlockReason::WaitChild(waited)) = parent.state {
            if waited == child_pid {
                parent.state = ProcessState::Ready;
            }
        }
        parent.pending_signals.raise(Signal::SIGCHLD);
    }
}
