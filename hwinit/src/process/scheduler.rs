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

use super::context::CpuContext;
use super::signals::Signal;
use super::{BlockReason, Process, ProcessState, MAX_PROCESSES, PROCESS_KERNEL_STACK_SIZE};
use crate::cpu::gdt::{KERNEL_CS, KERNEL_DS};
use crate::memory::{global_registry_mut, is_registry_initialized, PAGE_SIZE};
use crate::serial::{put_hex32, puts};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::cpu::per_cpu::MAX_CPUS;

// Per-AP idle contexts. When an AP descheduled a user process and nothing
// else is runnable, we MUST NOT return the outgoing process's ring-3 context.
// That would iretq into user code with current_pid=0, corrupting everything.
// Instead we return a ring-0 context pointing at the AP idle HLT loop.
static mut AP_IDLE_CTX: [CpuContext; MAX_CPUS] = [const { CpuContext::empty() }; MAX_CPUS];

/// Ring 0 HLT loop for idle APs. The LAPIC timer will fire and
/// scheduler_tick will pick a real process if one becomes Ready.
/// Never called directly — only entered via iretq from the ISR.
#[inline(never)]
unsafe fn ap_idle_hlt_loop() -> ! {
    loop {
        core::arch::asm!("sti; hlt; cli", options(nostack, nomem));
    }
}

/// Build and return a kernel-mode idle context for the given AP core.
/// Uses the AP's boot kernel stack and jumps to ap_idle_hlt_loop.
unsafe fn ap_idle_context(core_idx: u32) -> &'static CpuContext {
    let ctx = &mut AP_IDLE_CTX[core_idx as usize];
    let pcpu = crate::cpu::per_cpu::current();
    ctx.rip = ap_idle_hlt_loop as u64;
    ctx.rsp = pcpu.boot_kernel_rsp;
    ctx.cs = KERNEL_CS as u64;
    ctx.ss = KERNEL_DS as u64;
    ctx.rflags = 0x202; // IF=1
    // zero all GPRs — clean state
    ctx.rax = 0; ctx.rbx = 0; ctx.rcx = 0; ctx.rdx = 0;
    ctx.rsi = 0; ctx.rdi = 0; ctx.rbp = 0;
    ctx.r8 = 0; ctx.r9 = 0; ctx.r10 = 0; ctx.r11 = 0;
    ctx.r12 = 0; ctx.r13 = 0; ctx.r14 = 0; ctx.r15 = 0;
    ctx
}
// GLOBAL STATE

/// The flat process table.  Index == PID.
/// Protected by PROCESS_TABLE_LOCK for SMP safety.
pub(crate) static mut PROCESS_TABLE: [Option<Process>; MAX_PROCESSES] =
    [const { None }; MAX_PROCESSES];

/// Spinlock guarding PROCESS_TABLE mutations from concurrent cores.
/// The timer ISR on every core calls scheduler_tick() which must not
/// race against itself or syscall handlers modifying the same process.
/// Not reentrant — never acquire from code that already holds it.
/// Disables IF on acquire, restores on release — safe from both ISR
/// and non-ISR contexts (BSP idle loop, task manager, etc).
pub(crate) static PROCESS_TABLE_LOCK: crate::sync::IsrSafeRawSpinLock =
    crate::sync::IsrSafeRawSpinLock::new();

/// PID of the currently executing process on core 0 (BSP).
/// For SMP, each core tracks its own current_pid via gs:[0x0C].
/// This atomic remains for compatibility with code that reads it
/// from non-ISR contexts (syscall handlers run on the core that
/// triggered the syscall, so they use per-CPU reads).
static CURRENT_PID: AtomicU32 = AtomicU32::new(0);

/// Monotonically increasing counter of scheduler ticks (= timer IRQ count).
static TICK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Total number of live processes (including PID 0).
static LIVE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Number of processes with a TSC-based deadline (Sleep or FutexWait with timeout).
/// Used to skip the `wake_expired_sleepers` scan when no deadlines are active.
static TIMED_BLOCK_COUNT: AtomicU32 = AtomicU32::new(0);

/// TSC value at the moment the kernel (PID 0) called `mark_kernel_hlt()` — i.e.
/// when it entered the idle HLT path.  Zero = kernel is not currently in HLT.
/// Written by the kernel event loop; read+cleared by the timer ISR.
static KERNEL_HLT_ENTRY_TSC: AtomicU64 = AtomicU64::new(0);

/// Monotonically-increasing total TSC cycles the kernel has spent in HLT idle.
/// Exposed via `SysInfo::idle_tsc` so userspace can compute absolute CPU%.
static IDLE_TSC_TOTAL: AtomicU64 = AtomicU64::new(0);

/// True while every PID-0 quantum has ended in HLT (kernel has no real work).
/// Stays true across multiple user-process quanta; cleared only when PID 0
/// completes a full quantum WITHOUT calling `mark_kernel_hlt()` (i.e. real
/// work arrived — keyboard event, signal, etc.).
/// `pick_next` uses this to skip PID 0 for as long as the kernel is idle,
/// giving user processes consecutive quanta instead of the forced 50/50 split.
static KERNEL_LAST_WAS_IDLE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// How many consecutive quanta PID 0 has been skipped by the idle-donation
/// logic.  When this reaches `MAX_KERNEL_SKIP` the reservation overrides the
/// skip, forcing PID 0 to run — guaranteeing a kernel CPU floor.
static KERNEL_SKIP_STREAK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Kernel is guaranteed at least 1 quantum per (MAX_KERNEL_SKIP + 1).
/// 1 → kernel floor ≈ 50%. This keeps input polling responsive without
/// starving user processes under the idle-donation path.
const MAX_KERNEL_SKIP: u32 = 1;

/// True once `init_scheduler()` has been called.
static mut SCHEDULER_READY: bool = false;

/// Kernel CR3 (PML4 physical address).  Set once during `init_scheduler()`.
/// Needed so syscall/ISR handlers can switch to kernel page tables when
/// the buddy allocator must traverse identity-mapped physical addresses
/// that may overlap with user-space virtual mappings.
static mut KERNEL_CR3: u64 = 0;

/// Return the kernel's CR3 (PML4 physical address).  0 before scheduler init.
pub fn get_kernel_cr3() -> u64 {
    unsafe { KERNEL_CR3 }
}

/// TSC frequency in Hz — set by `set_tsc_frequency()` during platform init.
/// Used to convert millisecond sleep durations to TSC deadlines.
static mut TSC_FREQUENCY: u64 = 0;

// Per-CPU fields for next_cr3 and current_fpu_ptr are accessed via
// gs:[0x10] and gs:[0x18] respectively.  See cpu/per_cpu.rs.
// No more extern "C" globals — those lived in .data and were single-core only.

/// Helper: read this core's current PID from per-CPU data (GS-relative).
/// Falls back to CURRENT_PID atomic before per-CPU is initialized.
#[inline(always)]
unsafe fn this_core_pid() -> u32 {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::current_pid()
    } else {
        CURRENT_PID.load(Ordering::Relaxed)
    }
}

/// Helper: set this core's current PID.
#[inline(always)]
unsafe fn set_this_core_pid(pid: u32) {
    CURRENT_PID.store(pid, Ordering::SeqCst); // backward compat
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::set_current_pid(pid);
    }
}

/// Helper: get sequential core index of the calling CPU (0 = BSP).
#[inline(always)]
unsafe fn this_core_index() -> u32 {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        crate::cpu::per_cpu::current_core_index()
    } else {
        0
    }
}

/// Helper: write next_cr3 into per-CPU data for the ISR to read.
#[inline(always)]
unsafe fn set_percpu_next_cr3(cr3: u64) {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.next_cr3 = cr3;
    }
}

/// Helper: write current_fpu_ptr into per-CPU data for the ISR to read.
#[inline(always)]
unsafe fn set_percpu_fpu_ptr(ptr: u64) {
    if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
        let pcpu = crate::cpu::per_cpu::current();
        pcpu.current_fpu_ptr = ptr;
    }
}

// PUBLIC INFO SNAPSHOT (allocation-free, for the task manager)

/// A cheap, copyable snapshot of one process's status for display.
#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: [u8; 32],
    pub state: ProcessState,
    pub cpu_ticks: u64,
    /// Accumulated TSC cycles this process was actively running (excluding HLT idle).
    pub cpu_tsc: u64,
    pub pages_alloc: u64,
    pub priority: u8,
}

impl ProcessInfo {
    /// Create a zeroed ProcessInfo (used to pre-fill arrays).
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

    /// Get the process name as a byte slice (up to first NUL).
    pub fn name_bytes(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        &self.name[..end]
    }
}

// SCHEDULER HANDLE (zero-size, all methods are statics)

/// Handle to the global scheduler.  Obtain via `SCHEDULER`.
pub struct Scheduler;

/// The single global scheduler instance.
pub static SCHEDULER: Scheduler = Scheduler;

// TSC FREQUENCY (set once, read by sleep computation)

/// Store the TSC frequency for sleep deadline computation.
///
/// Call once from platform init, after TSC calibration.
///
/// # Safety
/// Single-threaded init.
pub unsafe fn set_tsc_frequency(freq: u64) {
    TSC_FREQUENCY = freq;
}

/// Get the stored TSC frequency (Hz).  Returns 0 if not yet calibrated.
pub fn tsc_frequency() -> u64 {
    unsafe { TSC_FREQUENCY }
}

/// Record the TSC at the instant the kernel enters the HLT idle path.
///
/// Call immediately BEFORE the `sti; hlt; cli` sequence in the kernel event
/// loop whenever the PS/2 poll returns nothing.  The timer ISR will split the
/// kernel's scheduler quantum into active work time and HLT idle time so that
/// `SysInfo::cpu_tsc` reflects true CPU utilization, not relative shares.
pub fn mark_kernel_hlt() {
    KERNEL_HLT_ENTRY_TSC.store(crate::cpu::tsc::read_tsc(), Ordering::Relaxed);
}

/// Total TSC cycles the kernel has spent halted in HLT idle since boot.
/// Exposed via `SysInfo::idle_tsc`.
pub fn idle_tsc_total() -> u64 {
    IDLE_TSC_TOTAL.load(Ordering::Relaxed)
}

/// Increment the timed-block counter (called when a process enters sleep or
/// futex-wait with a deadline).
pub fn inc_timed_block_count() {
    TIMED_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed);
}

impl Scheduler {
    /// Snapshot the process table for display (e.g. task manager).
    ///
    /// Fills `out` with up to `out.len()` entries and returns how many were
    /// written.  No allocation.
    /// This broke me, fuck this schedular but somehow it "works"
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

    /// Number of currently live processes.
    pub fn live_count(&self) -> u32 {
        LIVE_COUNT.load(Ordering::Relaxed)
    }

    /// Current PID (on the calling core).
    pub fn current_pid(&self) -> u32 {
        unsafe { this_core_pid() }
    }

    /// Total scheduler ticks since boot.
    pub fn tick_count(&self) -> u32 {
        TICK_COUNT.load(Ordering::Relaxed)
    }

    /// Mutable reference to the current process's fd table.
    ///
    /// # Safety
    /// Call only with interrupts disabled (e.g. from a syscall handler).
    pub unsafe fn current_fd_table_mut(&self) -> &'static mut morpheus_helix::vfs::FdTable {
        let pid = this_core_pid() as usize;
        &mut PROCESS_TABLE[pid].as_mut().unwrap().fd_table
    }

    /// Mutable reference to the current process descriptor.
    ///
    /// # Safety
    /// Call only with interrupts disabled (e.g. from a syscall handler).
    pub unsafe fn current_process_mut(&self) -> &'static mut Process {
        let pid = this_core_pid() as usize;
        PROCESS_TABLE[pid].as_mut().unwrap()
    }

    /// Returns a mutable reference to the memory leader process of the current thread.
    /// If the current process is a thread, returns the thread group leader. Otherwise,
    /// returns the current process itself.
    /// # Safety
    /// Call only with interrupts disabled.
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

    /// Returns a mutable reference to the memory leader process by PID.
    /// # Safety
    /// Call only with interrupts disabled.
    pub unsafe fn memory_leader_mut_by_pid(&self, pid: u32) -> Option<&'static mut Process> {
        let p = PROCESS_TABLE.get(pid as usize)?.as_ref()?;
        let leader_pid = if p.thread_group_leader != 0 {
            p.thread_group_leader as usize
        } else {
            pid as usize
        };
        PROCESS_TABLE.get_mut(leader_pid)?.as_mut()
    }

    /// Immutable reference to a process by PID.
    ///
    /// # Safety
    /// Single-threaded; call only with interrupts disabled.
    pub unsafe fn process_by_pid(&self, pid: u32) -> Option<&'static Process> {
        PROCESS_TABLE.get(pid as usize).and_then(|s| s.as_ref())
    }

    /// Send a signal to a process by PID.
    /// Returns `Err` if the PID is not found or not alive.
    pub unsafe fn send_signal(&self, pid: u32, sig: Signal) -> Result<(), &'static str> {
        PROCESS_TABLE_LOCK.lock();
        let result = self.send_signal_inner(pid, sig);
        PROCESS_TABLE_LOCK.unlock();
        result
    }

    /// Inner send_signal — caller must hold PROCESS_TABLE_LOCK.
    /// Used by paths already holding the lock (e.g. terminate_process_inner).
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

        // SIGKILL and SIGSTOP are delivered immediately without process consent bc i do not fuck arround.
        // no but bc its an exokernel and my mindset for this is absolute controll and zero hidden state thus
        // i will not allow a process to live after the kernel said it should die.
        // THERE is no blocking io or whatever the kernel kills a process frees its pages and cleans up after it.
        // the process is self responsible if it makes the kernel want to kill it its its own fault and morpheus wont wait.
        // TODO: Clean up after dead process :)
        match sig {
            Signal::SIGKILL => {
                // if the target is running a syscall on another core, it holds
                // &mut Process via current_process_mut() WITHOUT the lock.
                // calling terminate_process_inner here would race on the same
                // Process entry. defer: set pending signal, let the target
                // core's scheduler_tick deliver it after saving context.
                if slot.running_on != u32::MAX {
                    slot.pending_signals.raise(Signal::SIGKILL);
                } else {
                    puts("[SCHED] SIGKILL → PID ");
                    put_hex32(pid);
                    puts("\n");
                    terminate_process_inner(slot, -9);
                }
            }
            Signal::SIGSTOP => {
                // same race concern as SIGKILL — defer if running on another core.
                if slot.running_on != u32::MAX {
                    slot.pending_signals.raise(Signal::SIGSTOP);
                } else {
                    slot.state = ProcessState::Blocked(BlockReason::Io);
                }
            }
            Signal::SIGCONT => {
                if let ProcessState::Blocked(_) = slot.state {
                    slot.state = ProcessState::Ready;
                }
            }
            other => {
                slot.pending_signals.raise(other);
                // Unblock the process if it's waiting on stdin so the
                // signal can be delivered promptly (e.g. Ctrl+C → SIGINT
                // should interrupt a blocking read immediately).
                if matches!(slot.state, ProcessState::Blocked(BlockReason::StdinRead)) {
                    slot.state = ProcessState::Ready;
                }
            }
        }
        Ok(())
    }

    /// Set the scheduling priority of a process.
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

    /// Get the scheduling priority of a process.
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
    kernel_proc.pid = 0;
    kernel_proc.set_name("kernel");
    kernel_proc.state = ProcessState::Running;
    kernel_proc.priority = 0; // highest priority
    kernel_proc.running_on = 0; // running on BSP

    // Read the current CR3 — the kernel shares the UEFI identity map.
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    kernel_proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;
    KERNEL_CR3 = kernel_proc.cr3;

    PROCESS_TABLE[0] = Some(kernel_proc);
    LIVE_COUNT.store(1, Ordering::SeqCst);
    set_this_core_pid(0);
    SCHEDULER_READY = true;

    // Point this core's PerCpu FPU pointer at PID 0's FpuState so the very
    // first timer tick FXSAVE's into the right place.
    if let Some(p) = PROCESS_TABLE[0].as_mut() {
        let fpu_ptr = &mut p.fpu_state as *mut super::context::FpuState as u64;
        set_percpu_fpu_ptr(fpu_ptr);
    }

    puts("[SCHED] initialized — kernel is PID 0\n");
}

/// Spawn a new kernel-mode thread at `entry_fn`.
///
/// Returns the new PID, or `Err` if the table is full or setup fails.
///
/// # Safety
/// Scheduler must be initialized!(dont be like me and debug for hours until you find out the schedular isnt setup).  `entry_fn` must be a valid function
/// that runs in Ring 0 and eventually calls `exit_process()`.
pub unsafe fn spawn_kernel_thread(
    name: &str,
    entry_fn: u64,
    priority: u8,
) -> Result<u32, &'static str> {
    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    PROCESS_TABLE_LOCK.lock();

    // Find a free slot.
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

    // Inherit kernel CR3 (kernel threads share the address space).
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
    proc.cr3 = cr3 & 0x000F_FFFF_FFFF_F000;

    // Allocate a private kernel stack.
    if let Err(e) = proc.alloc_kernel_stack() {
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    // Set up the initial context so the scheduler can `iretq` into entry_fn.
    proc.context = CpuContext::new_kernel_thread(
        entry_fn,
        proc.kernel_stack_top,
        KERNEL_CS as u64,
        KERNEL_DS as u64,
    );

    let _ = (pid, entry_fn);
    crate::serial::log_info("SCHED", 770, "kernel thread spawned");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    PROCESS_TABLE_LOCK.unlock();
    Ok(pid)
}

/// Terminate the calling process with the given exit code.
///
/// Transitions the current process to `Zombie` and yields to the scheduler.
/// The scheduler will pick another ready process.
///
/// # Safety
/// Must be called from within a process context (not the timer ISR itself xD).
pub unsafe fn exit_process(code: i32) -> ! {
    let pid = this_core_pid();

    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        terminate_process_inner(proc, code);
    }
    PROCESS_TABLE_LOCK.unlock();
    // lock released before sti — timer ISR on this core can now safely acquire it
    core::arch::asm!("sti", options(nostack, nomem));
    loop {
        core::arch::asm!("hlt", options(nostack, nomem));
    }
}

/// Inner helper: mark process as Zombie with the given exit code.
/// Wakes the parent if it is blocked on WaitChild for this process.
unsafe fn terminate_process_inner(proc: &mut Process, code: i32) {
    let child_pid = proc.pid;
    let parent_pid = proc.parent_pid;

    crate::syscall::handler::release_fb_lock_if_holder(child_pid);

    proc.state = ProcessState::Zombie;
    proc.exit_code = Some(code);
    LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);

    puts("[SCHED] PID ");
    put_hex32(child_pid);
    puts(" exited code=");
    put_hex32(code as u32);
    puts("\n");

    // Wake parent if blocked on WaitChild(this_pid) and send SIGCHLD.
    if let Some(Some(parent)) = PROCESS_TABLE.get_mut(parent_pid as usize) {
        if let ProcessState::Blocked(BlockReason::WaitChild(waited)) = parent.state {
            if waited == child_pid {
                parent.state = ProcessState::Ready;
            }
        }
        parent.pending_signals.raise(Signal::SIGCHLD);
    }
}

// SCHEDULER TICK (called from timer ISR — Phase 4)

/// Called from the timer ISR (ASM, MS x64 ABI) on every tick, on EVERY core.
///
/// Saves the outgoing process context from the ISR stack frame and switches
/// to the next Ready process.  Returns the `CpuContext` of the next process
/// so the ISR can restore it via the iretq frame patch.
///
/// SMP-safe: acquires PROCESS_TABLE_LOCK, uses per-CPU data for current_pid,
/// next_cr3, current_fpu_ptr.  Each core independently picks a process
/// to run, skipping any process already running on another core.
///
/// # Safety
/// Must only be called from the timer ISR with interrupts disabled.
#[no_mangle]
pub unsafe extern "C" fn scheduler_tick(current_ctx: &CpuContext) -> &'static CpuContext {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    let now_tsc = crate::cpu::tsc::read_tsc();
    let core_idx = this_core_index();

    // BSP-only: auto-present framebuffer and poll PS/2 mouse.
    // these touch hardware that only the BSP should access.
    if core_idx == 0 {
        crate::syscall::handler::fb_present_tick();
        crate::ps2_mouse::poll();
    }

    // acquire process table lock
    PROCESS_TABLE_LOCK.lock();

    let cur_pid = this_core_pid() as usize;

    // ── AP idle fast path ────────────────────────────────────────────────
    // PID 0 belongs to the BSP. if an AP is idle (cur_pid==0), we must NOT
    // save/restore PID 0's context — doing so races with the BSP's ISR
    // and corrupts the BSP's RIP/RSP, teleporting it into the AP's HLT loop.
    // instead: check for real work, switch if found, otherwise return the
    // ISR stack frame unchanged so the AP continues its HLT loop.
    if cur_pid == 0 && core_idx != 0 {
        deliver_pending_signals(0);
        wake_expired_sleepers();

        let next_pid = pick_next(0, true, core_idx);
        if next_pid == 0 {
            // no user process available — return to ring 0 idle loop.
            set_percpu_fpu_ptr(0);
            set_percpu_next_cr3(KERNEL_CR3);
            // restore AP's own kernel stack in TSS for future ring transitions
            let pcpu = crate::cpu::per_cpu::current();
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
            pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
            PROCESS_TABLE_LOCK.unlock();
            return ap_idle_context(core_idx);
        }

        // found a real process — switch AP to it
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
            let fpu_ptr = &mut next.fpu_state as *mut super::context::FpuState as u64;
            set_percpu_fpu_ptr(fpu_ptr);
            &next.context
        } else {
            // pick_next lied — PID doesn't exist. reset to idle.
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

    // ── Normal path (BSP, or AP running a real process) ──────────────────

    // idle-donation tracking (BSP only, PID 0 only)
    let hlt_entry = KERNEL_HLT_ENTRY_TSC.swap(0, Ordering::Relaxed);
    let kernel_was_idle = cur_pid == 0 && hlt_entry != 0 && core_idx == 0;

    if cur_pid == 0 && core_idx == 0 {
        KERNEL_LAST_WAS_IDLE.store(kernel_was_idle, Ordering::Relaxed);
    }

    // save context of currently running process
    if let Some(Some(cur)) = PROCESS_TABLE.get_mut(cur_pid) {
        cur.context = *current_ctx;

        // fix SS RPL for user-mode processes
        if cur.context.cs & 3 == 3 {
            cur.context.ss |= 3;
        }

        cur.cpu_ticks += 1;

        // TSC-based accounting
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
        // clear running_on — this process is being descheduled
        cur.running_on = u32::MAX;
    }

    // signal delivery + sleeper wakeup (any core can do this)
    deliver_pending_signals(cur_pid as u32);
    wake_expired_sleepers();

    // pick next process for THIS core
    let compositor_active = crate::syscall::handler::compositor_active();
    let skip_kernel = if compositor_active || core_idx != 0 {
        // APs never run PID 0 (kernel). only BSP runs it.
        // in compositor mode BSP also doesn't skip PID 0.
        core_idx != 0
    } else {
        KERNEL_LAST_WAS_IDLE.load(Ordering::Relaxed)
    };
    let next_pid = pick_next(cur_pid, skip_kernel, core_idx);

    // AP ran a user process but nothing else is runnable — return to idle.
    // CRITICAL: must NOT return the outgoing process's ring-3 context.
    // That would iretq into user code while AP thinks it's PID 0 — instant UB.
    // Instead, return a kernel-mode idle context on the AP's own stack.
    if next_pid == 0 && core_idx != 0 {
        set_this_core_pid(0);
        set_percpu_fpu_ptr(0);
        set_percpu_next_cr3(KERNEL_CR3);
        // restore AP's own kernel stack in TSS
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

        // update kernel stack pointers for ring 3 → ring 0 transitions
        if next.kernel_stack_top != 0 {
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, next.kernel_stack_top);
            // update per-CPU kernel_syscall_rsp
            if crate::cpu::per_cpu::AP_ONLINE_COUNT.load(Ordering::Relaxed) > 0 {
                let pcpu = crate::cpu::per_cpu::current();
                pcpu.kernel_syscall_rsp = next.kernel_stack_top;
            }
        }

        // tell the ISR ASM which CR3 to load
        if crate::memory::is_valid_cr3(next.cr3) {
            set_percpu_next_cr3(next.cr3);
        }

        // point the ISR's FXRSTOR at the incoming process's FPU area
        let fpu_ptr = &mut next.fpu_state as *mut super::context::FpuState as u64;
        set_percpu_fpu_ptr(fpu_ptr);

        &next.context
    } else {
        // pick_next returned a PID that no longer exists. recover gracefully.
        if core_idx != 0 {
            // AP: go back to idle
            set_this_core_pid(0);
            set_percpu_fpu_ptr(0);
            set_percpu_next_cr3(KERNEL_CR3);
            let pcpu = crate::cpu::per_cpu::current();
            crate::cpu::gdt::set_kernel_stack_for_core(core_idx, pcpu.boot_kernel_rsp);
            pcpu.kernel_syscall_rsp = pcpu.boot_kernel_rsp;
            PROCESS_TABLE_LOCK.unlock();
            return ap_idle_context(core_idx);
        }
        // BSP: reset core PID and return kernel context
        set_this_core_pid(0);
        set_percpu_fpu_ptr(0);
        set_percpu_next_cr3(KERNEL_CR3);
        if let Some(Some(cur)) = PROCESS_TABLE.get(cur_pid) {
            &cur.context
        } else {
            // nothing — return ISR stack frame unchanged
            core::mem::transmute::<&CpuContext, &'static CpuContext>(current_ctx)
        }
    };

    PROCESS_TABLE_LOCK.unlock();

    result
}
// USER PROCESS SPAWN
/// Returns the new thread's PID (which also serves as the TID).
pub unsafe fn spawn_user_thread(entry: u64, stack_top: u64, arg: u64) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};

    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    PROCESS_TABLE_LOCK.lock();

    let parent_pid = this_core_pid();
    let parent = match PROCESS_TABLE
        .get(parent_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => p,
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return Err("no current process");
        }
    };
    let parent_cr3 = parent.cr3;
    let parent_mmap_brk = parent.mmap_brk;
    let parent_cwd = parent.cwd;
    let parent_cwd_len = parent.cwd_len;

    // Determine thread group leader: if parent is already a thread, inherit
    // its leader.  Otherwise, parent IS the leader.
    let group_leader = if parent.thread_group_leader != 0 {
        parent.thread_group_leader
    } else {
        parent_pid
    };

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

    let tid = slot_idx as u32;

    PROCESS_TABLE[slot_idx] = Some(Process::empty());
    let thread = PROCESS_TABLE[slot_idx].as_mut().ok_or_else(|| {
        PROCESS_TABLE_LOCK.unlock();
        "failed to initialize thread slot"
    })?;

    thread.pid = tid;
    thread.set_name("thread");
    thread.parent_pid = parent_pid;
    thread.priority = 128;
    thread.state = ProcessState::Ready;
    thread.cr3 = parent_cr3;
    thread.thread_group_leader = group_leader;
    thread.mmap_brk = parent_mmap_brk;
    thread.cwd = parent_cwd;
    thread.cwd_len = parent_cwd_len;

    // Own kernel stack for interrupt/syscall entry from Ring 3.
    if let Err(e) = thread.alloc_kernel_stack() {
        PROCESS_TABLE[slot_idx] = None;
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    // Ring 3 entry: rip=entry, rsp=stack_top-8, rdi=arg.
    //
    // SysV x86-64 ABI requires RSP ≡ 8 (mod 16) at function entry
    // (as if a `call` pushed the return address).  `iretq` doesn't
    // push a return address, so we pre-adjust RSP by -8.
    thread.context = CpuContext {
        rip: entry,
        rsp: stack_top - 8,
        rdi: arg,
        rflags: 0x202, // IF=1
        cs: USER_CS as u64,
        ss: USER_DS as u64,
        ..CpuContext::empty()
    };

    puts("[SCHED] spawned TID ");
    put_hex32(tid);
    puts(" group=");
    put_hex32(group_leader);
    puts("\n");

    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
    PROCESS_TABLE_LOCK.unlock();
    Ok(tid)
}

/// Spawn a Ring 3 user process from an ELF64 binary.
///
/// The binary is parsed, loaded into a fresh address space (with kernel
/// mappings cloned), and added to the process table as Ready.
///
/// If `inherit_fds` is true, the child gets a copy of the parent's fd table
/// (with pipe refcounts incremented).  Otherwise, it gets an empty fd table.
///
/// If `arg_blob` is non-empty, the null-separated arg strings are stored
/// in the child's `args` buffer for retrieval via `SYS_GETARGS`.
///
/// # Safety
/// Scheduler, paging, and MemoryRegistry must all be initialized.
pub unsafe fn spawn_user_process(
    name: &str,
    elf_data: &[u8],
    arg_blob: &[u8],
    arg_count: u8,
    inherit_fds: bool,
) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};
    use crate::elf::{load_elf64, USER_STACK_TOP};

    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    let (image, page_table) = load_elf64(elf_data).map_err(|e| {
        use crate::elf::ElfError;
        use crate::serial::puts;
        puts("[SCHED] ELF load error: ");
        match e {
            ElfError::TooSmall => puts("too small\n"),
            ElfError::BadMagic => puts("bad magic\n"),
            ElfError::Not64Bit => puts("not 64-bit\n"),
            ElfError::NotLittleEndian => puts("not little-endian\n"),
            ElfError::NotX86_64 => puts("not x86-64\n"),
            ElfError::NotExecutable => puts("not executable (e_type)\n"),
            ElfError::BadPhdr => puts("bad program header\n"),
            ElfError::NoLoadSegments => puts("no PT_LOAD segments\n"),
            ElfError::MapFailed => puts("page mapping failed\n"),
            ElfError::AllocFailed => puts("physical page alloc failed\n"),
        }
        "ELF load failed"
    })?;

    // elf loaded, pages allocated. now lock the process table for slot search + write.
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
    proc.priority = 128;
    proc.state = ProcessState::Ready;
    proc.cr3 = page_table.pml4_phys;

    // Allocate a per-process kernel stack (for interrupts from Ring 3).
    if let Err(e) = proc.alloc_kernel_stack() {
        PROCESS_TABLE_LOCK.unlock();
        return Err(e);
    }

    // fd inheritance
    if inherit_fds {
        let parent_pid = proc.parent_pid as usize;
        if let Some(Some(parent)) = PROCESS_TABLE.get(parent_pid) {
            proc.fd_table = parent.fd_table;
            // Increment pipe refcounts for each inherited pipe fd.
            //
            // This is needed for SHELL PIPELINE processes only (e.g. `cmd1 | cmd2`).
            // Previously it also handled compositor-spawned children, which inherited
            // a pipe read-end at fd 0 for their stdin.  That pipe-based stdin path
            // was replaced by per-process input buffers (SYS_FORWARD_INPUT), so
            // composited clients no longer have any pipes in their inherited fd_table.
            // The loop runs but bumps zero refcounts for them — pure no-op overhead.
            //
            // Dedup by (pipe_idx, direction) — if two fds point to the same
            // pipe reader (e.g. fd 0 and fd 3 both have O_PIPE_READ for
            // pipe 0), we only bump that refcount once.  But a reader and a
            // writer for the same pipe ARE distinct refcounts and both must
            // be bumped.  The old code used a single seen_pipes[idx] bitmap
            // which silently ate writer bumps when a reader for the same
            // pipe was already seen.  That was a fun afternoon.
            use morpheus_helix::types::open_flags;
            let mut seen_readers: [bool; 256] = [false; 256];
            let mut seen_writers: [bool; 256] = [false; 256];
            for fd_desc in proc.fd_table.fds.iter() {
                if fd_desc.is_open() {
                    let fl = fd_desc.flags;
                    let idx = fd_desc.mount_idx as usize;
                    if idx < 256 {
                        if fl & open_flags::O_PIPE_READ != 0 && !seen_readers[idx] {
                            crate::pipe::pipe_add_reader(fd_desc.mount_idx);
                            seen_readers[idx] = true;
                        }
                        if fl & open_flags::O_PIPE_WRITE != 0 && !seen_writers[idx] {
                            crate::pipe::pipe_add_writer(fd_desc.mount_idx);
                            seen_writers[idx] = true;
                        }
                    }
                }
            }
        }
    }

    // argv blob
    if !arg_blob.is_empty() && arg_count > 0 {
        let len = arg_blob.len().min(256);
        proc.args[..len].copy_from_slice(&arg_blob[..len]);
        proc.args_len = len as u16;
        proc.argc = arg_count;
    }

    // Set up Ring 3 entry context.
    //
    // SysV x86-64 ABI requires RSP ≡ 8 (mod 16) at function entry
    // (as if a `call` pushed the return address).  `iretq` doesn't
    // push a return address, so we pre-adjust RSP by -8.
    proc.context = CpuContext {
        rip: image.entry,
        rsp: USER_STACK_TOP - 8,
        rflags: 0x202, // IF=1
        cs: USER_CS as u64,
        ss: USER_DS as u64,
        ..CpuContext::empty()
    };

    // Register ELF segments (code + data + stack) in VMA table so that
    // free_process_resources can free all owned physical pages without
    // walking leaf page-table entries (which may include MMIO addresses).
    for seg in &image.segments {
        let pages = seg.memsz / PAGE_SIZE;
        let _ = proc.vma_table.insert(seg.vaddr, seg.phys, pages, true);
    }

    // Tally allocated pages.
    let total_pages: u64 = image.segments.iter().map(|s| s.memsz / 4096).sum();
    proc.pages_allocated = total_pages;

    let _ = (pid, image.entry, proc.cr3);
    crate::serial::log_info("SCHED", 771, "user process spawned");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);

    PROCESS_TABLE_LOCK.unlock();
    Ok(pid)
}

// BLOCKING PRIMITIVES (called from syscall handlers)

/// Block the current process until a TSC deadline, then yield to the scheduler.
///
/// The process is marked `Blocked(Sleep(deadline))`.  The timer ISR will
/// eventually unblock it (via `wake_expired_sleepers`) and resume it.
///
/// # Safety
/// Must be called from a syscall handler with interrupts disabled.
pub unsafe fn block_sleep(deadline: u64) -> u64 {
    let pid = this_core_pid() as usize;
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid) {
        proc.state = ProcessState::Blocked(BlockReason::Sleep(deadline));
        TIMED_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    PROCESS_TABLE_LOCK.unlock();

    // STI + HLT is atomic on x86: no interrupt window between them.
    // The timer ISR saves our context, switches away, and later resumes us
    // here once the sleep deadline has expired and we're marked Ready again.
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
    0
}

/// Wait for a child process to exit.
///
/// If the child is already a Zombie, reaps immediately and returns its exit
/// code.  Otherwise, blocks the caller with `BlockReason::WaitChild(pid)`.
///
/// Returns the child's exit code (as u64), or a negative errno on error.
///
/// # Safety
/// Must be called from a syscall handler with interrupts disabled.
pub unsafe fn wait_for_child(child_pid: u32) -> u64 {
    let current = this_core_pid();

    PROCESS_TABLE_LOCK.lock();

    // Validate: does the child exist?
    let (child_parent, child_state) = match PROCESS_TABLE
        .get(child_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => (p.parent_pid, p.state),
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return u64::MAX - 3; // ESRCH
        }
    };

    // Validate: is it actually our child?
    if child_parent != current {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 3; // ESRCH
    }

    // Already reaped?
    if matches!(child_state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 10; // ECHILD
    }

    // Already zombie? Reap now (under lock — reap_child acquires GLOBAL_REGISTRY).
    if child_state == ProcessState::Zombie {
        let result = reap_child(child_pid);
        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    // Block until the child exits.
    let cur = current as usize;
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(cur) {
        proc.state = ProcessState::Blocked(BlockReason::WaitChild(child_pid));
    }

    PROCESS_TABLE_LOCK.unlock();

    // Yield — resume when terminate_process_inner unblocks us.
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));

    // Child should now be Zombie (terminate_process_inner set it).
    // re-acquire lock for reap
    PROCESS_TABLE_LOCK.lock();
    let result = reap_child(child_pid);
    PROCESS_TABLE_LOCK.unlock();
    result
}

/// Non-blocking wait: reap if zombie, return EAGAIN if still running.
///
/// Returns the exit code if the child was a zombie (reaps it), or
/// `EAGAIN` (u64::MAX - 11) if the child is still running.
pub unsafe fn try_wait_child(child_pid: u32) -> u64 {
    let current = this_core_pid();

    PROCESS_TABLE_LOCK.lock();

    let (child_parent, child_state) = match PROCESS_TABLE
        .get(child_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => (p.parent_pid, p.state),
        None => {
            PROCESS_TABLE_LOCK.unlock();
            return u64::MAX - 3; // ESRCH
        }
    };

    if child_parent != current {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 3; // ESRCH
    }

    if matches!(child_state, ProcessState::Terminated) {
        PROCESS_TABLE_LOCK.unlock();
        return u64::MAX - 10; // ECHILD — already reaped
    }

    if child_state == ProcessState::Zombie {
        let result = reap_child(child_pid);
        PROCESS_TABLE_LOCK.unlock();
        return result;
    }

    // Still running — return EAGAIN.
    PROCESS_TABLE_LOCK.unlock();
    u64::MAX - 11
}

/// Reap a Zombie child: extract its exit code, free resources, mark Terminated.
unsafe fn reap_child(pid: u32) -> u64 {
    if let Some(Some(child)) = PROCESS_TABLE.get_mut(pid as usize) {
        let code = child.exit_code.unwrap_or(-1);

        free_process_resources(child);

        child.state = ProcessState::Terminated;

        puts("[SCHED] reaped PID ");
        put_hex32(pid);
        puts("\n");

        code as u64
    } else {
        u64::MAX - 10 // ECHILD
    }
}

/// Release physical resources held by a process.
///
/// Frees the kernel stack, then all owned physical pages tracked in the VMA
/// table, then walks the page-table hierarchy to free intermediate table
/// pages only (never leaf pages — those are handled by the VMA loop above).
unsafe fn free_process_resources(proc: &mut Process) {
    // free kernel stack
    if proc.kernel_stack_base != 0 && is_registry_initialized() {
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(PAGE_SIZE);
        let mut registry = global_registry_mut();
        let _ = registry.free_pages(proc.kernel_stack_base, pages);
        proc.kernel_stack_base = 0;
        proc.kernel_stack_top = 0;
    }

    // Free per-process compositor FB surface (not tracked in VMA table as
    // owned, because sys_map_phys records owns_phys=false).
    if proc.fb_surface_phys != 0 && proc.fb_surface_pages != 0 && is_registry_initialized() {
        let mut registry = global_registry_mut();
        let _ = registry.free_pages(proc.fb_surface_phys, proc.fb_surface_pages);
        proc.fb_surface_phys = 0;
        proc.fb_surface_pages = 0;
        proc.fb_surface_dirty = false;
    }

    // Free all owned physical pages via VMA table.
    // This covers ELF code/data segments, the user stack, and any
    // SYS_MMAP allocations that were not munmap'd before exit.
    // Non-owned VMAs (MAP_PHYS / FB_MAP) are skipped — their physical
    // addresses are MMIO or shared memory, not buddy-allocator RAM.
    if is_registry_initialized() {
        let mut registry = global_registry_mut();
        for (_, vma) in proc.vma_table.iter() {
            if vma.owns_phys {
                let _ = registry.free_pages(vma.phys, vma.pages);
            }
        }
    }

    // free user page tables (if this isn't the kernel process and not a thread)
    // Threads share their leader's CR3 — freeing it would nuke the parent.
    if proc.cr3 != 0 && proc.pid != 0 && proc.thread_group_leader == 0 {
        let kernel_cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) kernel_cr3, options(nostack, nomem));
        let kernel_cr3 = kernel_cr3 & 0x000F_FFFF_FFFF_F000;

        if proc.cr3 != kernel_cr3 {
            free_user_page_tables(proc.cr3);
            proc.cr3 = 0;
        }
    }
}

/// Walk a PML4 and free all user-owned **intermediate page-table pages**.
///
/// Only the lower half (PML4 indices 0..256) is walked — upper half is kernel.
/// At every level, only entries with the USER bit are ours (allocated by
/// `ensure_user_table`).
///
/// **Leaf physical pages are NOT freed here.**  They are freed separately
/// via the VMA table in `free_process_resources`.  This avoids the entire
/// class of bugs where MMIO or shared-memory physical addresses (from
/// MAP_PHYS / FB_MAP) are fed into the buddy allocator and corrupt its
/// free lists.
unsafe fn free_user_page_tables(pml4_phys: u64) {
    if !is_registry_initialized() {
        return;
    }
    let mut registry = global_registry_mut();

    let pml4 = pml4_phys as *const u64;

    const PRESENT: u64 = 1 << 0;
    const USER: u64 = 1 << 2;
    const HUGE: u64 = 1 << 7;
    const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

    for pml4_idx in 0..256usize {
        let pml4e = *pml4.add(pml4_idx);
        if pml4e & PRESENT == 0 || pml4e & USER == 0 {
            continue;
        }
        let pdpt_phys = pml4e & ADDR_MASK;
        let pdpt = pdpt_phys as *const u64;

        for pdpt_idx in 0..512usize {
            let pdpte = *pdpt.add(pdpt_idx);
            if pdpte & PRESENT == 0 || pdpte & USER == 0 {
                continue;
            }
            if pdpte & HUGE != 0 {
                // 1 GiB huge leaf — skip (freed via VMA table)
                continue;
            }
            let pd_phys = pdpte & ADDR_MASK;
            let pd = pd_phys as *const u64;

            for pd_idx in 0..512usize {
                let pde = *pd.add(pd_idx);
                if pde & PRESENT == 0 || pde & USER == 0 {
                    continue;
                }
                if pde & HUGE != 0 {
                    // 2 MiB huge leaf — skip (freed via VMA table)
                    continue;
                }
                let pt_phys = pde & ADDR_MASK;
                // Do NOT iterate PT leaf entries — all leaf physical pages
                // are freed via the VMA table.  Only free the PT page itself.
                let _ = registry.free_pages(pt_phys, 1);
            }
            let _ = registry.free_pages(pd_phys, 1);
        }
        let _ = registry.free_pages(pdpt_phys, 1);
    }
    let _ = registry.free_pages(pml4_phys, 1);
}

// HELPERS

/// Unblock any processes whose sleep deadline has been reached.
///
/// Called from `scheduler_tick` on every timer interrupt — must be fast.
/// PERF: Early-exits if no processes have active deadlines.
unsafe fn wake_expired_sleepers() {
    if TIMED_BLOCK_COUNT.load(Ordering::Relaxed) == 0 {
        return;
    }
    let now = crate::cpu::tsc::read_tsc();
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        match proc.state {
            ProcessState::Blocked(BlockReason::Sleep(deadline)) => {
                if now >= deadline {
                    proc.state = ProcessState::Ready;
                    TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                }
            }
            ProcessState::Blocked(BlockReason::FutexWait(_)) => {
                if proc.futex_deadline != 0 && now >= proc.futex_deadline {
                    proc.state = ProcessState::Ready;
                    proc.futex_deadline = 0;
                    TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

/// Round-robin: scan PROCESS_TABLE from `(current+1)..MAX` then wrap.
/// Returns current if nothing else is runnable.
///
/// SMP-aware: skips any process already `running_on` another core.
/// APs (core_idx != 0) never pick PID 0 — the kernel runs exclusively on BSP.
///
/// When `skip_kernel` is true (kernel just HLT'd), PID 0 is excluded from the
/// first pass so user processes receive consecutive quanta while the kernel has
/// nothing to do.  PID 0 remains the hard fallback if no other process is ready.
///
/// Hard floor: even when `skip_kernel` is true, PID 0 is forced to run once
/// every `MAX_KERNEL_SKIP + 1` quanta via `KERNEL_SKIP_STREAK`.  This prevents
/// CPU-hungry user processes from starving the kernel entirely.
unsafe fn pick_next(current: usize, skip_kernel: bool, core_idx: u32) -> usize {
    let n = MAX_PROCESSES;
    let is_bsp = core_idx == 0;

    // enforce kernel floor (BSP only)
    let streak = KERNEL_SKIP_STREAK.load(Ordering::Relaxed);
    let skip_kernel = if is_bsp && skip_kernel && streak >= MAX_KERNEL_SKIP {
        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
        false // floor hit — give PID 0 a turn
    } else {
        skip_kernel
    };

    // first pass: find any ready process not running on another core
    for delta in 1..=n {
        let candidate = (current + delta) % n;
        // APs never touch PID 0. BSP skips it when kernel was idle.
        if candidate == 0 && (!is_bsp || skip_kernel) {
            continue;
        }
        if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
            // already running on another core — hands off
            if p.running_on != u32::MAX {
                continue;
            }
            if p.state == ProcessState::Ready {
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

    // second pass (fallback): include PID 0 (BSP only)
    if is_bsp {
        KERNEL_SKIP_STREAK.store(0, Ordering::Relaxed);
        for delta in 1..=n {
            let candidate = (current + delta) % n;
            if let Some(Some(p)) = PROCESS_TABLE.get(candidate) {
                if p.running_on != u32::MAX {
                    continue;
                }
                if p.state == ProcessState::Ready {
                    return candidate;
                }
            }
        }
    }

    // nothing else runnable — stay on current if it's still alive
    if let Some(Some(p)) = PROCESS_TABLE.get(current) {
        if p.state.is_runnable() && !(current == 0 && !is_bsp) {
            return current;
        }
    }
    // absolute fallback: BSP → PID 0. AP → 0 (scheduler_tick handles it).
    0
}

/// Deliver the highest-priority pending signal to the given PID.
unsafe fn deliver_pending_signals(pid: u32) {
    let proc = match PROCESS_TABLE.get_mut(pid as usize).and_then(|s| s.as_mut()) {
        Some(p) => p,
        None => return,
    };

    // Don't deliver new signals while already inside a signal handler.
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
            // SIG_IGN — ignore this signal.
            continue;
        }
        if handler > 1 {
            // User-registered handler — redirect execution.
            // Save the current context so SYS_SIGRETURN can restore it.
            proc.saved_signal_context = proc.context;
            proc.saved_signal_fpu = proc.fpu_state;
            proc.in_signal_handler = true;

            // SysV x86-64 ABI: RSP must be 16-byte aligned, then -8 for
            // the missing return address (as if `call` had pushed it).
            // Push 0 as return address — handler MUST call sigreturn().
            let aligned_rsp = (proc.context.rsp & !0xF) - 8;
            proc.context.rip = handler;
            proc.context.rdi = sig as u8 as u64;
            proc.context.rsp = aligned_rsp;

            return; // One signal at a time; rest stay pending.
        }
        // handler == 0 (SIG_DFL)
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

// WAKE FUNCTIONS (called from producers to unblock waiting processes)

/// Wake all processes blocked on `BlockReason::StdinRead`.
///
/// Called by the keyboard input path after pushing bytes to the stdin buffer.
pub unsafe fn wake_stdin_waiters() {
    PROCESS_TABLE_LOCK.lock();
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if matches!(proc.state, ProcessState::Blocked(BlockReason::StdinRead)) {
            proc.state = ProcessState::Ready;
        }
    }
    PROCESS_TABLE_LOCK.unlock();
}

/// Wake all processes blocked on `BlockReason::PipeRead(idx)`.
///
/// Called after writing to a pipe so that blocked readers can proceed.
pub unsafe fn wake_pipe_readers(pipe_idx: u8) {
    PROCESS_TABLE_LOCK.lock();
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if let ProcessState::Blocked(BlockReason::PipeRead(idx)) = proc.state {
            if idx == pipe_idx {
                proc.state = ProcessState::Ready;
            }
        }
    }
    PROCESS_TABLE_LOCK.unlock();
}

/// Wake a process blocked on `BlockReason::InputRead` for the given PID.
///
/// Called by `SYS_FORWARD_INPUT` after the compositor pushes bytes into a
/// child's per-process input buffer.  Only wakes the specific target — no
/// reason to iterate 64 processes when we know exactly who we're after.
pub unsafe fn wake_input_reader(pid: u32) {
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        if matches!(proc.state, ProcessState::Blocked(BlockReason::InputRead)) {
            proc.state = ProcessState::Ready;
        }
    }
    PROCESS_TABLE_LOCK.unlock();
}

/// Wake up to `count` processes blocked on `FutexWait(addr)`.
///
/// Returns number actually woken.  If count == u32::MAX, wakes all.
pub unsafe fn wake_futex_waiters(addr: u64, count: u32) -> u32 {
    PROCESS_TABLE_LOCK.lock();
    let mut woken = 0u32;
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if woken >= count {
            break;
        }
        if let ProcessState::Blocked(BlockReason::FutexWait(wait_addr)) = proc.state {
            if wait_addr == addr {
                // If this waiter had a deadline, decrement the timed-block counter
                // since wake_expired_sleepers will no longer need to expire it.
                if proc.futex_deadline != 0 {
                    proc.futex_deadline = 0;
                    TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                }
                proc.state = ProcessState::Ready;
                woken += 1;
            }
        }
    }
    PROCESS_TABLE_LOCK.unlock();
    woken
}
