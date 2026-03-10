use super::lifecycle::terminate_process_inner;
use crate::process::{
    BlockReason, Process, ProcessPolicyClass, ProcessPowerMode, ProcessState, Signal,
    MAX_PROCESSES,
};
use crate::cpu::per_cpu::MAX_CPUS;
use crate::serial::{put_hex32, puts};
use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerSystemState {
    PerfBoost = 0,
    Balanced = 1,
    EcoBias = 2,
    ThermalGuard = 3,
    ThermalEmergency = 4,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerCoreState {
    Active = 0,
    LightIdle = 1,
    DeepIdleEligible = 2,
    Parked = 3,
    UnparkPending = 4,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerTransitionReason {
    TickIdle = 0,
    TickWork = 1,
    WakeWork = 2,
    HysteresisPark = 3,
    HysteresisUnpark = 4,
    ThermalGuard = 5,
    ThermalEmergency = 6,
}

pub(crate) static mut PROCESS_TABLE: [Option<Process>; MAX_PROCESSES] =
    [const { None }; MAX_PROCESSES];

pub(crate) static PROCESS_TABLE_LOCK: crate::sync::IsrSafeRawSpinLock =
    crate::sync::IsrSafeRawSpinLock::new();

static CURRENT_PID: AtomicU32 = AtomicU32::new(0);
pub(super) static TICK_COUNT: AtomicU32 = AtomicU32::new(0);
pub(super) static LIVE_COUNT: AtomicU32 = AtomicU32::new(0);
pub(super) static TIMED_BLOCK_COUNT: AtomicU32 = AtomicU32::new(0);
pub(super) static EARLIEST_DEADLINE: AtomicU64 = AtomicU64::new(u64::MAX);
pub(super) static KERNEL_HLT_ENTRY_TSC: AtomicU64 = AtomicU64::new(0);
pub(super) static IDLE_TSC_TOTAL: AtomicU64 = AtomicU64::new(0);
pub(super) static KERNEL_LAST_WAS_IDLE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
pub(super) static KERNEL_SKIP_STREAK: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub(super) const MAX_KERNEL_SKIP: u32 = 1;
pub(super) static SCHED_SYSTEM_STATE: AtomicU8 = AtomicU8::new(SchedulerSystemState::Balanced as u8);
pub(super) static PER_CORE_STATE: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(SchedulerCoreState::Active as u8) }; MAX_CPUS];
pub(super) static PER_CORE_LOAD_EWMA: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_IDLE_TICKS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_IDLE_ENTRY_TSC: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_IDLE_ACCUM_TSC: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_LAST_ACTIVE_TSC: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_PARK_CANDIDATE: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_IDLE_STREAK: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_ACTIVE_STREAK: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_LAST_STATE_CHANGE_TSC: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
pub(super) static PER_CORE_LAST_TRANSITION_REASON: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(SchedulerTransitionReason::TickWork as u8) }; MAX_CPUS];
pub(super) static SCHED_TIER_HITS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
pub(super) static THERMAL_LEVEL: AtomicU8 = AtomicU8::new(0);
pub(super) static THERMAL_CONFIDENCE: AtomicU8 = AtomicU8::new(0);
pub(super) static THERMAL_SOURCE_TSC: AtomicU64 = AtomicU64::new(0);

pub(super) static mut SCHEDULER_READY: bool = false;
pub(super) static mut KERNEL_CR3: u64 = 0;
pub(super) static mut TSC_FREQUENCY: u64 = 0;

// lock-light waiter indexes for fast wake paths.
static STDIN_WAITERS: AtomicU64 = AtomicU64::new(0);
static INPUT_WAITERS: AtomicU64 = AtomicU64::new(0);
static PIPE_WAITERS: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];
static FUTEX_WAITERS: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];

#[inline(always)]
fn pid_mask(pid: u32) -> u64 {
    if pid < 64 { 1u64 << pid } else { 0 }
}

#[inline(always)]
fn futex_bucket(addr: u64) -> usize {
    ((addr >> 2) as usize) & 0xFF
}

pub(crate) fn mark_stdin_waiter(pid: u32) {
    STDIN_WAITERS.fetch_or(pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn clear_stdin_waiter(pid: u32) {
    STDIN_WAITERS.fetch_and(!pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn stdin_waiters_mask() -> u64 {
    STDIN_WAITERS.load(Ordering::Relaxed)
}

pub(crate) fn mark_input_waiter(pid: u32) {
    INPUT_WAITERS.fetch_or(pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn clear_input_waiter(pid: u32) {
    INPUT_WAITERS.fetch_and(!pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn input_waiters_mask() -> u64 {
    INPUT_WAITERS.load(Ordering::Relaxed)
}

pub(crate) fn mark_pipe_waiter(pid: u32, pipe_idx: u8) {
    PIPE_WAITERS[pipe_idx as usize].fetch_or(pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn clear_pipe_waiter(pid: u32, pipe_idx: u8) {
    PIPE_WAITERS[pipe_idx as usize].fetch_and(!pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn pipe_waiters_mask(pipe_idx: u8) -> u64 {
    PIPE_WAITERS[pipe_idx as usize].load(Ordering::Relaxed)
}

pub(crate) fn mark_futex_waiter(pid: u32, addr: u64) {
    FUTEX_WAITERS[futex_bucket(addr)].fetch_or(pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn clear_futex_waiter(pid: u32, addr: u64) {
    FUTEX_WAITERS[futex_bucket(addr)].fetch_and(!pid_mask(pid), Ordering::Relaxed);
}

pub(crate) fn futex_waiters_mask(addr: u64) -> u64 {
    FUTEX_WAITERS[futex_bucket(addr)].load(Ordering::Relaxed)
}

pub(crate) fn clear_waiter_all(pid: u32) {
    let mask = !pid_mask(pid);
    STDIN_WAITERS.fetch_and(mask, Ordering::Relaxed);
    INPUT_WAITERS.fetch_and(mask, Ordering::Relaxed);
    for b in PIPE_WAITERS.iter() {
        b.fetch_and(mask, Ordering::Relaxed);
    }
    for b in FUTEX_WAITERS.iter() {
        b.fetch_and(mask, Ordering::Relaxed);
    }
}

/// Proactive stale waiter cleanup. Clears waiter bits for processes that are no longer
/// actually blocked on that resource. Low cost when called infrequently (every ~1k ticks).
/// Prevents bit accumulation over long system uptime.
pub(crate) unsafe fn cleanup_stale_waiters() {
    let mut stdin_mask = STDIN_WAITERS.load(Ordering::Relaxed);
    let mut input_mask = INPUT_WAITERS.load(Ordering::Relaxed);
    let mut stale_stdin = 0u64;
    let mut stale_input = 0u64;

    // detect stale stdin waiter bits
    while stdin_mask != 0 {
        let bit = stdin_mask.trailing_zeros() as u32;
        stdin_mask &= stdin_mask - 1;
        if let Some(Some(p)) = PROCESS_TABLE.get(bit as usize) {
            if !matches!(p.state, ProcessState::Blocked(BlockReason::StdinRead)) {
                stale_stdin |= 1u64 << bit;
            }
        } else {
            stale_stdin |= 1u64 << bit;
        }
    }

    // detect stale input waiter bits
    while input_mask != 0 {
        let bit = input_mask.trailing_zeros() as u32;
        input_mask &= input_mask - 1;
        if let Some(Some(p)) = PROCESS_TABLE.get(bit as usize) {
            if !matches!(p.state, ProcessState::Blocked(BlockReason::InputRead)) {
                stale_input |= 1u64 << bit;
            }
        } else {
            stale_input |= 1u64 << bit;
        }
    }

    if stale_stdin != 0 {
        STDIN_WAITERS.fetch_and(!stale_stdin, Ordering::Relaxed);
    }
    if stale_input != 0 {
        INPUT_WAITERS.fetch_and(!stale_input, Ordering::Relaxed);
    }

    // scan pipe waiter array
    for (pipe_idx, waiter_set) in PIPE_WAITERS.iter().enumerate() {
        let mut mask = waiter_set.load(Ordering::Relaxed);
        let mut stale = 0u64;
        while mask != 0 {
            let bit = mask.trailing_zeros() as u32;
            mask &= mask - 1;
            if let Some(Some(p)) = PROCESS_TABLE.get(bit as usize) {
                if let ProcessState::Blocked(BlockReason::PipeRead(idx)) = p.state {
                    if idx as usize != pipe_idx {
                        stale |= 1u64 << bit;
                    }
                } else {
                    stale |= 1u64 << bit;
                }
            } else {
                stale |= 1u64 << bit;
            }
        }
        if stale != 0 {
            waiter_set.fetch_and(!stale, Ordering::Relaxed);
        }
    }

    // scan futex waiter array
    for waiter_set in FUTEX_WAITERS.iter() {
        let mut mask = waiter_set.load(Ordering::Relaxed);
        let mut stale = 0u64;
        while mask != 0 {
            let bit = mask.trailing_zeros() as u32;
            mask &= mask - 1;
            if let Some(Some(p)) = PROCESS_TABLE.get(bit as usize) {
                if !matches!(p.state, ProcessState::Blocked(BlockReason::FutexWait(_))) {
                    stale |= 1u64 << bit;
                }
            } else {
                stale |= 1u64 << bit;
            }
        }
        if stale != 0 {
            waiter_set.fetch_and(!stale, Ordering::Relaxed);
        }
    }
}

pub fn get_kernel_cr3() -> u64 {
    unsafe { KERNEL_CR3 }
}

pub fn scheduler_system_state() -> SchedulerSystemState {
    match SCHED_SYSTEM_STATE.load(Ordering::Relaxed) {
        0 => SchedulerSystemState::PerfBoost,
        1 => SchedulerSystemState::Balanced,
        2 => SchedulerSystemState::EcoBias,
        3 => SchedulerSystemState::ThermalGuard,
        _ => SchedulerSystemState::ThermalEmergency,
    }
}

pub fn set_scheduler_system_state(state: SchedulerSystemState) {
    SCHED_SYSTEM_STATE.store(state as u8, Ordering::Relaxed);
}

pub fn refresh_balanced_system_mode() {
    // Thermal hooks own state selection when confidence is non-zero.
    if THERMAL_CONFIDENCE.load(Ordering::Relaxed) != 0 {
        return;
    }
    set_scheduler_system_state(SchedulerSystemState::Balanced);
}

#[inline(always)]
pub fn record_tier_hit(tier: usize) {
    if tier < SCHED_TIER_HITS.len() {
        SCHED_TIER_HITS[tier].fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(always)]
pub fn update_core_load_ewma(core_idx: u32, ready_count: u32) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    // ewma = (7/8 * old) + (1/8 * sample)
    let old = PER_CORE_LOAD_EWMA[idx].load(Ordering::Relaxed);
    let ewma = old.saturating_mul(7).saturating_add(ready_count) >> 3;
    PER_CORE_LOAD_EWMA[idx].store(ewma, Ordering::Relaxed);
}

#[inline(always)]
pub fn set_core_state(core_idx: u32, state: SchedulerCoreState) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    PER_CORE_STATE[idx].store(state as u8, Ordering::Relaxed);
}

#[inline(always)]
pub fn core_state(core_idx: u32) -> SchedulerCoreState {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return SchedulerCoreState::Active;
    }
    match PER_CORE_STATE[idx].load(Ordering::Relaxed) {
        0 => SchedulerCoreState::Active,
        1 => SchedulerCoreState::LightIdle,
        2 => SchedulerCoreState::DeepIdleEligible,
        3 => SchedulerCoreState::Parked,
        _ => SchedulerCoreState::UnparkPending,
    }
}

pub fn transition_core_state(
    core_idx: u32,
    target: SchedulerCoreState,
    now_tsc: u64,
    reason: SchedulerTransitionReason,
) -> bool {
    const MIN_STATE_RESIDENCY_TSC: u64 = 250_000;
    const MIN_UNPARK_PENDING_TSC: u64 = 25_000;

    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return false;
    }

    let current = core_state(core_idx);
    if current == target {
        return true;
    }

    // force a staging state when waking a parked core.
    if current == SchedulerCoreState::Parked && target == SchedulerCoreState::Active {
        PER_CORE_STATE[idx].store(SchedulerCoreState::UnparkPending as u8, Ordering::Relaxed);
        PER_CORE_LAST_STATE_CHANGE_TSC[idx].store(now_tsc, Ordering::Relaxed);
        PER_CORE_LAST_TRANSITION_REASON[idx].store(
            SchedulerTransitionReason::HysteresisUnpark as u8,
            Ordering::Relaxed,
        );
        return false;
    }

    let last_tsc = PER_CORE_LAST_STATE_CHANGE_TSC[idx].load(Ordering::Relaxed);
    let min_residency = if current == SchedulerCoreState::UnparkPending {
        MIN_UNPARK_PENDING_TSC
    } else {
        MIN_STATE_RESIDENCY_TSC
    };

    if now_tsc.saturating_sub(last_tsc) < min_residency
        && reason != SchedulerTransitionReason::ThermalEmergency
    {
        return false;
    }

    PER_CORE_STATE[idx].store(target as u8, Ordering::Relaxed);
    PER_CORE_LAST_STATE_CHANGE_TSC[idx].store(now_tsc, Ordering::Relaxed);
    PER_CORE_LAST_TRANSITION_REASON[idx].store(reason as u8, Ordering::Relaxed);
    true
}

#[inline(always)]
pub fn mark_core_idle_tick(core_idx: u32) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    PER_CORE_IDLE_TICKS[idx].fetch_add(1, Ordering::Relaxed);
}

#[inline(always)]
pub fn mark_core_idle_enter(core_idx: u32, now_tsc: u64) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    let entry = PER_CORE_IDLE_ENTRY_TSC[idx].load(Ordering::Relaxed);
    if entry == 0 {
        PER_CORE_IDLE_ENTRY_TSC[idx].store(now_tsc, Ordering::Relaxed);
    }
}

#[inline(always)]
pub fn mark_core_idle_exit(core_idx: u32, now_tsc: u64) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    let entry = PER_CORE_IDLE_ENTRY_TSC[idx].swap(0, Ordering::Relaxed);
    if entry != 0 {
        PER_CORE_IDLE_ACCUM_TSC[idx].fetch_add(now_tsc.saturating_sub(entry), Ordering::Relaxed);
    }
}

pub fn sample_idle_tsc_total(now_tsc: u64) -> u64 {
    let mut total = IDLE_TSC_TOTAL.load(Ordering::Relaxed);
    let mut idx = 0usize;
    while idx < MAX_CPUS {
        let accum = PER_CORE_IDLE_ACCUM_TSC[idx].load(Ordering::Relaxed);
        let entry = PER_CORE_IDLE_ENTRY_TSC[idx].load(Ordering::Relaxed);
        total = total.saturating_add(accum);
        if entry != 0 {
            total = total.saturating_add(now_tsc.saturating_sub(entry));
        }
        idx += 1;
    }
    total
}

pub fn sample_per_core_idle_tsc(now_tsc: u64, out: &mut [u64]) -> usize {
    let cpu_count = crate::cpu::per_cpu::cpu_count() as usize;
    let limit = core::cmp::min(core::cmp::min(cpu_count, MAX_CPUS), out.len());
    let mut idx = 0usize;
    while idx < limit {
        let accum = PER_CORE_IDLE_ACCUM_TSC[idx].load(Ordering::Relaxed);
        let entry = PER_CORE_IDLE_ENTRY_TSC[idx].load(Ordering::Relaxed);
        let mut total = accum;
        if entry != 0 {
            total = total.saturating_add(now_tsc.saturating_sub(entry));
        }
        out[idx] = total;
        idx += 1;
    }
    idx
}

#[inline(always)]
pub fn mark_core_active_tsc(core_idx: u32, now_tsc: u64) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }
    PER_CORE_LAST_ACTIVE_TSC[idx].store(now_tsc, Ordering::Relaxed);
}

pub fn set_thermal_signal(level: u8, source_tsc: u64, confidence: u8) {
    THERMAL_LEVEL.store(level, Ordering::Relaxed);
    THERMAL_CONFIDENCE.store(confidence, Ordering::Relaxed);
    THERMAL_SOURCE_TSC.store(source_tsc, Ordering::Relaxed);

    // 0 = normal, 1 = warm, 2 = high, 3+ = emergency
    let state = if confidence == 0 {
        SchedulerSystemState::Balanced
    } else if level >= 3 {
        SchedulerSystemState::ThermalEmergency
    } else if level >= 2 {
        SchedulerSystemState::ThermalGuard
    } else {
        SchedulerSystemState::Balanced
    };
    set_scheduler_system_state(state);
}

pub fn update_park_hysteresis(core_idx: u32, ready_count: u32) {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return;
    }

    let system_state = scheduler_system_state();
    let park_idle_threshold = match system_state {
        SchedulerSystemState::PerfBoost => 16,
        SchedulerSystemState::Balanced => 8,
        SchedulerSystemState::EcoBias => 4,
        SchedulerSystemState::ThermalGuard => 3,
        SchedulerSystemState::ThermalEmergency => 2,
    };
    let unpark_active_threshold = 2u32;

    if ready_count == 0 {
        let idle = PER_CORE_IDLE_STREAK[idx].fetch_add(1, Ordering::Relaxed) + 1;
        PER_CORE_ACTIVE_STREAK[idx].store(0, Ordering::Relaxed);
        if idle >= park_idle_threshold {
            PER_CORE_PARK_CANDIDATE[idx].store(1, Ordering::Relaxed);
        }
    } else {
        let active = PER_CORE_ACTIVE_STREAK[idx].fetch_add(1, Ordering::Relaxed) + 1;
        PER_CORE_IDLE_STREAK[idx].store(0, Ordering::Relaxed);
        if active >= unpark_active_threshold {
            PER_CORE_PARK_CANDIDATE[idx].store(0, Ordering::Relaxed);
        }
    }
}

#[inline(always)]
pub fn core_should_park(core_idx: u32) -> bool {
    let idx = core_idx as usize;
    if idx >= MAX_CPUS {
        return false;
    }
    PER_CORE_PARK_CANDIDATE[idx].load(Ordering::Relaxed) != 0
}

pub fn get_earliest_deadline() -> u64 {
    EARLIEST_DEADLINE.load(Ordering::Relaxed)
}

pub fn try_set_earliest_deadline(old: u64, new: u64) -> bool {
    EARLIEST_DEADLINE
        .compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

#[inline(always)]
pub(super) unsafe fn this_core_pid() -> u32 {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::current_pid()
    } else {
        CURRENT_PID.load(Ordering::Relaxed)
    }
}

#[inline(always)]
pub(super) unsafe fn set_this_core_pid(pid: u32) {
    CURRENT_PID.store(pid, Ordering::SeqCst);
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::set_current_pid(pid);
    }
}

#[inline(always)]
pub(super) unsafe fn this_core_index() -> u32 {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::current_core_index()
    } else {
        0
    }
}

#[inline(always)]
pub(super) unsafe fn set_percpu_next_cr3(cr3: u64) {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.next_cr3 = cr3;
    }
}

#[inline(always)]
pub(super) unsafe fn set_percpu_fpu_ptr(ptr: u64) {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.current_fpu_ptr = ptr;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: [u8; 32],
    pub state: ProcessState,
    pub cpu_ticks: u64,
    pub cpu_tsc: u64,
    pub pages_alloc: u64,
    pub priority: u8,
    pub importance_16: u8,
    pub power_mode: ProcessPowerMode,
    pub policy_class: ProcessPolicyClass,
    pub capability_bits: u32,
}

impl ProcessInfo {
    pub const fn zeroed() -> Self {
        Self {
            pid: 0,
            name: [0u8; 32],
            state: ProcessState::Ready,
            cpu_ticks: 0,
            cpu_tsc: 0,
            pages_alloc: 0,
            priority: 0,
            importance_16: 8,
            power_mode: ProcessPowerMode::Balanced,
            policy_class: ProcessPolicyClass::Throughput,
            capability_bits: 0,
        }
    }

    pub fn name_bytes(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        &self.name[..end]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SchedulerDebugInfo {
    pub system_state: SchedulerSystemState,
    pub tier_hits: [u64; 6],
    pub thermal_level: u8,
    pub thermal_confidence: u8,
    pub core_state: [u8; MAX_CPUS],
    pub core_last_transition_reason: [u8; MAX_CPUS],
    pub core_load_ewma: [u32; MAX_CPUS],
    pub core_park_candidate: [u8; MAX_CPUS],
}

impl SchedulerDebugInfo {
    pub const fn zeroed() -> Self {
        Self {
            system_state: SchedulerSystemState::Balanced,
            tier_hits: [0u64; 6],
            thermal_level: 0,
            thermal_confidence: 0,
            core_state: [0u8; MAX_CPUS],
            core_last_transition_reason: [0u8; MAX_CPUS],
            core_load_ewma: [0u32; MAX_CPUS],
            core_park_candidate: [0u8; MAX_CPUS],
        }
    }
}

pub struct Scheduler;

pub static SCHEDULER: Scheduler = Scheduler;

impl Scheduler {
    pub fn snapshot_processes(&self, out: &mut [ProcessInfo]) -> usize {
        let mut n = 0;
        unsafe {
            PROCESS_TABLE_LOCK.lock();
            for slot in PROCESS_TABLE.iter() {
                if n >= out.len() {
                    break;
                }
                if let Some(p) = slot {
                    if !p.is_free() {
                        out[n] = ProcessInfo {
                            pid: p.pid,
                            name: p.name,
                            state: p.state,
                            cpu_ticks: p.cpu_ticks,
                            cpu_tsc: p.cpu_tsc,
                            pages_alloc: p.pages_allocated,
                            priority: p.priority,
                            importance_16: p.importance_16,
                            power_mode: p.power_mode,
                            policy_class: p.policy_class,
                            capability_bits: p.capability_bits,
                        };
                        n += 1;
                    }
                }
            }
            PROCESS_TABLE_LOCK.unlock();
        }
        n
    }

    pub fn live_count(&self) -> u32 {
        LIVE_COUNT.load(Ordering::Relaxed)
    }

    pub fn current_pid(&self) -> u32 {
        unsafe { this_core_pid() }
    }

    pub fn tick_count(&self) -> u32 {
        TICK_COUNT.load(Ordering::Relaxed)
    }

    pub fn system_state(&self) -> SchedulerSystemState {
        scheduler_system_state()
    }

    pub fn set_system_state(&self, state: SchedulerSystemState) {
        set_scheduler_system_state(state);
    }

    pub fn thermal_signal(&self, level: u8, source_tsc: u64, confidence: u8) {
        set_thermal_signal(level, source_tsc, confidence);
    }

    pub fn tier_hits(&self) -> [u64; 6] {
        [
            SCHED_TIER_HITS[0].load(Ordering::Relaxed),
            SCHED_TIER_HITS[1].load(Ordering::Relaxed),
            SCHED_TIER_HITS[2].load(Ordering::Relaxed),
            SCHED_TIER_HITS[3].load(Ordering::Relaxed),
            SCHED_TIER_HITS[4].load(Ordering::Relaxed),
            SCHED_TIER_HITS[5].load(Ordering::Relaxed),
        ]
    }

    pub fn debug_snapshot(&self) -> SchedulerDebugInfo {
        let mut out = SchedulerDebugInfo::zeroed();
        out.system_state = scheduler_system_state();
        out.tier_hits = self.tier_hits();
        out.thermal_level = THERMAL_LEVEL.load(Ordering::Relaxed);
        out.thermal_confidence = THERMAL_CONFIDENCE.load(Ordering::Relaxed);
        let mut i = 0usize;
        while i < MAX_CPUS {
            out.core_state[i] = PER_CORE_STATE[i].load(Ordering::Relaxed);
            out.core_last_transition_reason[i] =
                PER_CORE_LAST_TRANSITION_REASON[i].load(Ordering::Relaxed);
            out.core_load_ewma[i] = PER_CORE_LOAD_EWMA[i].load(Ordering::Relaxed);
            out.core_park_candidate[i] = PER_CORE_PARK_CANDIDATE[i].load(Ordering::Relaxed);
            i += 1;
        }
        out
    }

    pub unsafe fn current_fd_table_mut(&self) -> &'static mut morpheus_helix::vfs::FdTable {
        let pid = this_core_pid() as usize;
        &mut PROCESS_TABLE[pid].as_mut().unwrap().fd_table
    }

    pub unsafe fn current_process_mut(&self) -> &'static mut Process {
        let pid = this_core_pid() as usize;
        PROCESS_TABLE[pid].as_mut().unwrap()
    }

    pub unsafe fn current_memory_leader_mut(&self) -> &'static mut Process {
        let pid = this_core_pid() as usize;
        let mut leader_pid = pid;
        if let Some(p) = PROCESS_TABLE[pid].as_ref() {
            if p.thread_group_leader != 0 {
                leader_pid = p.thread_group_leader as usize;
            }
        }
        PROCESS_TABLE[leader_pid].as_mut().unwrap()
    }

    pub unsafe fn memory_leader_mut_by_pid(&self, pid: u32) -> Option<&'static mut Process> {
        let p = PROCESS_TABLE.get(pid as usize)?.as_ref()?;
        let leader_pid = if p.thread_group_leader != 0 {
            p.thread_group_leader as usize
        } else {
            pid as usize
        };
        PROCESS_TABLE.get_mut(leader_pid)?.as_mut()
    }

    pub unsafe fn process_by_pid(&self, pid: u32) -> Option<&'static Process> {
        PROCESS_TABLE.get(pid as usize).and_then(|s| s.as_ref())
    }

    pub unsafe fn send_signal(&self, pid: u32, sig: Signal) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let result = self.send_signal_inner(pid, sig);
        PROCESS_TABLE_LOCK.unlock();
        result
    }

    pub(crate) unsafe fn send_signal_inner(
        &self,
        pid: u32,
        sig: Signal,
    ) -> Result<(), &'static str> {
        let slot = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
            Some(s) => s,
            None => return Err("send_signal: PID not found"),
        };

        if slot.is_free() {
            return Err("send_signal: process already terminated");
        }

        match sig {
            Signal::SIGKILL => {
                // running_on != MAX means another core may hold &mut Process.
                if slot.running_on != u32::MAX {
                    slot.pending_signals.raise(Signal::SIGKILL);
                } else {
                    puts("[SCHED] SIGKILL -> PID ");
                    put_hex32(pid);
                    puts("\n");
                    terminate_process_inner(slot, -9);
                }
            }
            Signal::SIGSTOP => {
                if slot.running_on != u32::MAX {
                    slot.pending_signals.raise(Signal::SIGSTOP);
                } else {
                    clear_waiter_all(pid);
                    slot.state = ProcessState::Blocked(BlockReason::Io);
                }
            }
            Signal::SIGCONT => {
                if let ProcessState::Blocked(_) = slot.state {
                    clear_waiter_all(pid);
                    slot.state = ProcessState::Ready;
                }
            }
            other => {
                slot.pending_signals.raise(other);
                if matches!(slot.state, ProcessState::Blocked(BlockReason::StdinRead)) {
                    clear_stdin_waiter(pid);
                    slot.state = ProcessState::Ready;
                }
            }
        }
        Ok(())
    }

    pub unsafe fn set_priority(&self, pid: u32, priority: u8) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("set_priority: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("set_priority: process terminated");
        }
        slot.priority = priority;
        PROCESS_TABLE_LOCK.unlock();
        Ok(())
    }

    pub unsafe fn set_importance(&self, pid: u32, importance_16: u8) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("set_importance: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("set_importance: process terminated");
        }
        slot.importance_16 = importance_16.clamp(1, 16);
        slot.effective_weight_cache = 0;
        PROCESS_TABLE_LOCK.unlock();
        Ok(())
    }

    pub unsafe fn set_power_mode(
        &self,
        pid: u32,
        power_mode: ProcessPowerMode,
    ) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("set_power_mode: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("set_power_mode: process terminated");
        }
        slot.power_mode = power_mode;
        slot.effective_weight_cache = 0;
        PROCESS_TABLE_LOCK.unlock();
        Ok(())
    }

    pub unsafe fn set_policy_class(
        &self,
        pid: u32,
        policy_class: ProcessPolicyClass,
    ) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("set_policy_class: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("set_policy_class: process terminated");
        }
        slot.policy_class = policy_class;
        slot.effective_weight_cache = 0;
        PROCESS_TABLE_LOCK.unlock();
        Ok(())
    }

    pub unsafe fn get_scheduler_policy(
        &self,
        pid: u32,
    ) -> Result<(u8, ProcessPowerMode, ProcessPolicyClass), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get(pid as usize).and_then(|s| s.as_ref()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("get_scheduler_policy: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("get_scheduler_policy: process terminated");
        }
        let out = (slot.importance_16, slot.power_mode, slot.policy_class);
        PROCESS_TABLE_LOCK.unlock();
        Ok(out)
    }

    pub unsafe fn get_priority(&self, pid: u32) -> Result<u8, &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let slot = match PROCESS_TABLE.get(pid as usize).and_then(|s| s.as_ref()) {
            Some(s) => s,
            None => {
                PROCESS_TABLE_LOCK.unlock();
                return Err("get_priority: PID not found");
            }
        };
        if slot.is_free() {
            PROCESS_TABLE_LOCK.unlock();
            return Err("get_priority: process terminated");
        }
        let prio = slot.priority;
        PROCESS_TABLE_LOCK.unlock();
        Ok(prio)
    }
}
