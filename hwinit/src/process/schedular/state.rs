use super::lifecycle::terminate_process_inner;
use crate::process::{BlockReason, Process, ProcessState, Signal, MAX_PROCESSES};
use crate::serial::{put_hex32, puts};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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
        }
    }

    pub fn name_bytes(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        &self.name[..end]
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
