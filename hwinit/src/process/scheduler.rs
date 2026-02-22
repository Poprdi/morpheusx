//! Round-robin process scheduler.
//!
//! The scheduler manages the `PROCESS_TABLE` (a fixed `[Option<Process>; 64]`
//! static array) and decides which process to run next on each timer tick.
//!
//! ## Current design: cooperative + preemptive
//!
//! - `scheduler_tick()` is called from the PIT timer ISR (Phase 4).
//!   It saves the current process context and picks the next Ready process.
//! - Processes may also call `SYS_YIELD` (syscall 3) to voluntarily give up
//!   the CPU early.
//!
//! ## Process 0 = kernel
//!
//! PID 0 represents the main kernel execution (the desktop / event loop).
//! It is always present and never sleeps indefinitely.  If no other process
//! is runnable, the scheduler returns to PID 0.
//!
//! ## No heap in the scheduler hot path
//!
//! Context switches must not allocate.  The `PROCESS_TABLE` is a static array;
//! the only heap usage is at process creation time (for the kernel stack).

use super::{
    Process, ProcessState, BlockReason, MAX_PROCESSES, PROCESS_KERNEL_STACK_SIZE,
};
use super::signals::Signal;
use super::context::CpuContext;
use crate::cpu::gdt::{KERNEL_CS, KERNEL_DS};
use crate::serial::{puts, put_hex32, put_hex64};
use core::sync::atomic::{AtomicU32, Ordering};

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL STATE
// ═══════════════════════════════════════════════════════════════════════════

/// The flat process table.  Index == PID.
static mut PROCESS_TABLE: [Option<Process>; MAX_PROCESSES] = {
    // Can't call Process::empty() in a const context with Option::None,
    // so we use a macro trick: all 64 slots start as None.
    [const { None }; MAX_PROCESSES]
};

/// PID of the currently executing process.
static CURRENT_PID: AtomicU32 = AtomicU32::new(0);

/// Monotonically increasing counter of scheduler ticks (= timer IRQ count).
static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Total number of live processes (including PID 0).
static LIVE_COUNT: AtomicU32 = AtomicU32::new(0);

/// True once `init_scheduler()` has been called.
static mut SCHEDULER_READY: bool = false;

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INFO SNAPSHOT (allocation-free, for the task manager)
// ═══════════════════════════════════════════════════════════════════════════

/// A cheap, copyable snapshot of one process's status for display.
#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo {
    pub pid:         u32,
    pub name:        [u8; 32],
    pub state:       ProcessState,
    pub cpu_ticks:   u64,
    pub pages_alloc: u64,
    pub priority:    u8,
}

// ═══════════════════════════════════════════════════════════════════════════
// SCHEDULER HANDLE (zero-size, all methods are statics)
// ═══════════════════════════════════════════════════════════════════════════

/// Handle to the global scheduler.  Obtain via `SCHEDULER`.
pub struct Scheduler;

/// The single global scheduler instance.
pub static SCHEDULER: Scheduler = Scheduler;

impl Scheduler {
    /// Snapshot the process table for display (e.g. task manager).
    ///
    /// Fills `out` with up to `out.len()` entries and returns how many were
    /// written.  No allocation.
    pub fn snapshot_processes(&self, out: &mut [ProcessInfo]) -> usize {
        let mut n = 0;
        unsafe {
            for slot in PROCESS_TABLE.iter() {
                if n >= out.len() { break; }
                if let Some(p) = slot {
                    if !p.is_free() {
                        out[n] = ProcessInfo {
                            pid:         p.pid,
                            name:        p.name,
                            state:       p.state,
                            cpu_ticks:   p.cpu_ticks,
                            pages_alloc: p.pages_allocated,
                            priority:    p.priority,
                        };
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Number of currently live processes.
    pub fn live_count(&self) -> u32 {
        LIVE_COUNT.load(Ordering::Relaxed)
    }

    /// Current PID.
    pub fn current_pid(&self) -> u32 {
        CURRENT_PID.load(Ordering::Relaxed)
    }

    /// Total scheduler ticks since boot.
    pub fn tick_count(&self) -> u32 {
        TICK_COUNT.load(Ordering::Relaxed)
    }

    /// Send a signal to a process by PID.
    /// Returns `Err` if the PID is not found or not alive.
    pub unsafe fn send_signal(&self, pid: u32, sig: Signal) -> Result<(), &'static str> {
        let slot = PROCESS_TABLE.get_mut(pid as usize)
            .and_then(|s| s.as_mut())
            .ok_or("send_signal: PID not found")?;

        if slot.is_free() {
            return Err("send_signal: process already terminated");
        }

        // SIGKILL and SIGSTOP are delivered immediately without process consent.
        match sig {
            Signal::SIGKILL => {
                puts("[SCHED] SIGKILL → PID ");
                put_hex32(pid);
                puts("\n");
                terminate_process_inner(slot, -9);
            }
            Signal::SIGSTOP => {
                slot.state = ProcessState::Blocked(BlockReason::Io);
            }
            Signal::SIGCONT => {
                if let ProcessState::Blocked(_) = slot.state {
                    slot.state = ProcessState::Ready;
                }
            }
            other => {
                slot.pending_signals.raise(other);
            }
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INIT
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the scheduler and create PID 0 (the kernel process).
///
/// Must be called once, after MemoryRegistry, GDT, IDT, and heap are ready.
///
/// # Safety
/// Single-threaded init; not reentrant.
pub unsafe fn init_scheduler() {
    if SCHEDULER_READY {
        puts("[SCHED] already initialized\n");
        return;
    }

    // Create PID 0 — the kernel itself.
    let mut kernel_proc = Process::empty();
    kernel_proc.pid   = 0;
    kernel_proc.set_name("kernel");
    kernel_proc.state = ProcessState::Running;
    kernel_proc.priority = 0; // highest priority

    // Read the current CR3 — the kernel shares the UEFI identity map.
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    kernel_proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    // PID 0's kernel stack is the one already in use — we don't allocate a
    // new one; just leave kernel_stack_top as 0 (unused for the running proc).

    PROCESS_TABLE[0] = Some(kernel_proc);
    LIVE_COUNT.store(1, Ordering::SeqCst);
    CURRENT_PID.store(0, Ordering::SeqCst);
    SCHEDULER_READY = true;

    puts("[SCHED] initialized — kernel is PID 0\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// SPAWN
// ═══════════════════════════════════════════════════════════════════════════

/// Spawn a new kernel-mode thread at `entry_fn`.
///
/// Returns the new PID, or `Err` if the table is full or setup fails.
///
/// # Safety
/// Scheduler must be initialized.  `entry_fn` must be a valid function
/// that runs in Ring 0 and eventually calls `exit_process()`.
pub unsafe fn spawn_kernel_thread(
    name: &str,
    entry_fn: u64,
    priority: u8,
) -> Result<u32, &'static str> {
    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    // Find a free slot.
    let slot_idx = (1..MAX_PROCESSES)
        .find(|&i| PROCESS_TABLE[i].as_ref().map(|p| p.is_free()).unwrap_or(true))
        .ok_or("process table full")?;

    let pid = slot_idx as u32;

    let mut proc = Process::empty();
    proc.pid      = pid;
    proc.set_name(name);
    proc.parent_pid = CURRENT_PID.load(Ordering::Relaxed);
    proc.priority = priority;
    proc.state    = ProcessState::Ready;

    // Inherit kernel CR3 (kernel threads share the address space).
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    // Allocate a private kernel stack.
    proc.alloc_kernel_stack()?;

    // Set up the initial context so the scheduler can `iretq` into entry_fn.
    proc.context = CpuContext::new_kernel_thread(
        entry_fn,
        proc.kernel_stack_top,
        KERNEL_CS as u64,
        KERNEL_DS as u64,
    );

    puts("[SCHED] spawned PID ");
    put_hex32(pid);
    puts(" \"");
    puts(proc.name_str());
    puts("\" entry=");
    put_hex64(entry_fn);
    puts("\n");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    Ok(pid)
}

// ═══════════════════════════════════════════════════════════════════════════
// EXIT
// ═══════════════════════════════════════════════════════════════════════════

/// Terminate the calling process with the given exit code.
///
/// Transitions the current process to `Zombie` and yields to the scheduler.
/// The scheduler will pick another ready process.
///
/// # Safety
/// Must be called from within a process context (not the timer ISR itself).
pub unsafe fn exit_process(code: i32) -> ! {
    let pid = CURRENT_PID.load(Ordering::Relaxed);
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        terminate_process_inner(proc, code);
    }
    // Yield — the scheduler will pick a different process.
    // We never return since our state is Zombie.
    loop { core::hint::spin_loop(); }
}

/// Inner helper: mark process as Zombie with the given exit code.
unsafe fn terminate_process_inner(proc: &mut Process, code: i32) {
    proc.state     = ProcessState::Zombie;
    proc.exit_code = Some(code);
    LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);

    puts("[SCHED] PID ");
    put_hex32(proc.pid);
    puts(" exited code=");
    put_hex32(code as u32);
    puts("\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// SCHEDULER TICK (called from timer ISR — Phase 4)
// ═══════════════════════════════════════════════════════════════════════════

/// Called from the timer ISR (ASM, MS x64 ABI) on every tick.
///
/// Saves the outgoing process context from the ISR stack frame and switches
/// to the next Ready process.  Returns the `CpuContext` of the next process
/// so the ISR can restore it via the iretq frame patch.
///
/// # Safety
/// Must only be called from the timer ISR with interrupts disabled.
#[no_mangle]
pub unsafe extern "C" fn scheduler_tick(current_ctx: &CpuContext) -> &'static CpuContext {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    let cur_pid = CURRENT_PID.load(Ordering::Relaxed) as usize;

    // Save context of currently running process.
    if let Some(Some(cur)) = PROCESS_TABLE.get_mut(cur_pid) {
        cur.context = *current_ctx;
        cur.cpu_ticks += 1;
        if cur.state == ProcessState::Running {
            cur.state = ProcessState::Ready;
        }
    }

    // Deliver any pending signals before picking next process.
    deliver_pending_signals(cur_pid as u32);

    // Pick next runnable process (round-robin, priority-weighted in future).
    let next_pid = pick_next(cur_pid);
    CURRENT_PID.store(next_pid as u32, Ordering::SeqCst);

    if let Some(Some(next)) = PROCESS_TABLE.get_mut(next_pid) {
        next.state = ProcessState::Running;
        &next.context
    } else {
        // Fallback: restore current (should not happen if PID 0 is always Ready).
        if let Some(Some(cur)) = PROCESS_TABLE.get(cur_pid) {
            &cur.context
        } else {
            panic!("scheduler: no runnable process")
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Round-robin: scan PROCESS_TABLE from `(current+1)..MAX` then wrap.
/// Returns current if nothing else is runnable.
unsafe fn pick_next(current: usize) -> usize {
    let n = MAX_PROCESSES;
    for delta in 1..=n {
        let candidate = (current + delta) % n;
        if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
            if p.state == ProcessState::Ready {
                return candidate;
            }
        }
    }
    // Nothing else runnable — stay on current (or fall back to PID 0).
    if let Some(Some(p)) = PROCESS_TABLE.get(current) {
        if p.state.is_runnable() {
            return current;
        }
    }
    // Absolute fallback: PID 0 is always alive.
    0
}

/// Deliver the highest-priority pending signal to the given PID.
unsafe fn deliver_pending_signals(pid: u32) {
    let proc = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
        Some(p) => p,
        None => return,
    };

    while let Some(sig) = proc.pending_signals.take_next() {
        match sig.default_action() {
            super::signals::SignalAction::Terminate => {
                puts("[SCHED] signal → PID ");
                put_hex32(pid);
                puts(" terminated\n");
                terminate_process_inner(proc, -(sig as u8 as i32));
                return; // Process is dead; stop delivering
            }
            super::signals::SignalAction::Stop => {
                proc.state = ProcessState::Blocked(BlockReason::Io);
                return;
            }
            super::signals::SignalAction::Continue => {
                if let ProcessState::Blocked(_) = proc.state {
                    proc.state = ProcessState::Ready;
                }
            }
            super::signals::SignalAction::Ignore => {}
        }
    }
}
