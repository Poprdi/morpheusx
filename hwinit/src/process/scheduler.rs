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
use crate::serial::{put_hex32, put_hex64, puts};
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

/// TSC frequency in Hz — set by `set_tsc_frequency()` during platform init.
/// Used to convert millisecond sleep durations to TSC deadlines.
static mut TSC_FREQUENCY: u64 = 0;

// CR3 of the next process to run.
// Written by `scheduler_tick()`, read by `irq_timer_isr` in ASM for address
// space switch.  Defined in `context_switch.s`.
extern "C" {
    static mut next_cr3: u64;
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INFO SNAPSHOT (allocation-free, for the task manager)
// ═══════════════════════════════════════════════════════════════════════════

/// A cheap, copyable snapshot of one process's status for display.
#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: [u8; 32],
    pub state: ProcessState,
    pub cpu_ticks: u64,
    pub pages_alloc: u64,
    pub priority: u8,
}

// ═══════════════════════════════════════════════════════════════════════════
// SCHEDULER HANDLE (zero-size, all methods are statics)
// ═══════════════════════════════════════════════════════════════════════════

/// Handle to the global scheduler.  Obtain via `SCHEDULER`.
pub struct Scheduler;

/// The single global scheduler instance.
pub static SCHEDULER: Scheduler = Scheduler;

// ═══════════════════════════════════════════════════════════════════════════
// TSC FREQUENCY (set once, read by sleep computation)
// ═══════════════════════════════════════════════════════════════════════════

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

impl Scheduler {
    /// Snapshot the process table for display (e.g. task manager).
    ///
    /// Fills `out` with up to `out.len()` entries and returns how many were
    /// written.  No allocation.
    pub fn snapshot_processes(&self, out: &mut [ProcessInfo]) -> usize {
        let mut n = 0;
        unsafe {
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
                            pages_alloc: p.pages_allocated,
                            priority: p.priority,
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

    /// Mutable reference to the current process's fd table.
    ///
    /// # Safety
    /// Single-threaded; call only with interrupts disabled (e.g. from a syscall handler).
    pub unsafe fn current_fd_table_mut(&self) -> &'static mut morpheus_helix::vfs::FdTable {
        let pid = CURRENT_PID.load(Ordering::Relaxed) as usize;
        &mut PROCESS_TABLE[pid].as_mut().unwrap().fd_table
    }

    /// Send a signal to a process by PID.
    /// Returns `Err` if the PID is not found or not alive.
    pub unsafe fn send_signal(&self, pid: u32, sig: Signal) -> Result<(), &'static str> {
        let slot = PROCESS_TABLE
            .get_mut(pid as usize)
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
    kernel_proc.pid = 0;
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
        .find(|&i| {
            PROCESS_TABLE[i]
                .as_ref()
                .map(|p| p.is_free())
                .unwrap_or(true)
        })
        .ok_or("process table full")?;

    let pid = slot_idx as u32;

    let mut proc = Process::empty();
    proc.pid = pid;
    proc.set_name(name);
    proc.parent_pid = CURRENT_PID.load(Ordering::Relaxed);
    proc.priority = priority;
    proc.state = ProcessState::Ready;

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
    loop {
        core::hint::spin_loop();
    }
}

/// Inner helper: mark process as Zombie with the given exit code.
/// Wakes the parent if it is blocked on WaitChild for this process.
unsafe fn terminate_process_inner(proc: &mut Process, code: i32) {
    let child_pid = proc.pid;
    let parent_pid = proc.parent_pid;

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

    // Wake any processes whose sleep deadline has expired.
    wake_expired_sleepers();

    // Pick next runnable process (round-robin, priority-weighted in future).
    let next_pid = pick_next(cur_pid);
    CURRENT_PID.store(next_pid as u32, Ordering::SeqCst);

    if let Some(Some(next)) = PROCESS_TABLE.get_mut(next_pid) {
        next.state = ProcessState::Running;

        // Update kernel stack pointers for Ring 3 → Ring 0 transitions.
        if next.kernel_stack_top != 0 {
            crate::cpu::gdt::set_kernel_stack(next.kernel_stack_top);
            extern "C" {
                static mut kernel_syscall_rsp: u64;
            }
            kernel_syscall_rsp = next.kernel_stack_top;
        }

        // Tell the ISR ASM which CR3 to load before iretq.
        next_cr3 = next.cr3;

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
// USER PROCESS SPAWN
// ═══════════════════════════════════════════════════════════════════════════

/// Spawn a Ring 3 user process from an ELF64 binary.
///
/// The binary is parsed, loaded into a fresh address space (with kernel
/// mappings cloned), and added to the process table as Ready.
///
/// # Safety
/// Scheduler, paging, and MemoryRegistry must all be initialized.
pub unsafe fn spawn_user_process(name: &str, elf_data: &[u8]) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};
    use crate::elf::{load_elf64, USER_STACK_TOP};

    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    let (image, page_table) = load_elf64(elf_data).map_err(|_| "ELF load failed")?;

    let slot_idx = (1..MAX_PROCESSES)
        .find(|&i| {
            PROCESS_TABLE[i]
                .as_ref()
                .map(|p| p.is_free())
                .unwrap_or(true)
        })
        .ok_or("process table full")?;

    let pid = slot_idx as u32;

    let mut proc = Process::empty();
    proc.pid = pid;
    proc.set_name(name);
    proc.parent_pid = CURRENT_PID.load(Ordering::Relaxed);
    proc.priority = 128;
    proc.state = ProcessState::Ready;
    proc.cr3 = page_table.pml4_phys;

    // Allocate a per-process kernel stack (for interrupts from Ring 3).
    proc.alloc_kernel_stack()?;

    // Set up Ring 3 entry context.
    proc.context = CpuContext {
        rip: image.entry,
        rsp: USER_STACK_TOP,
        rflags: 0x202, // IF=1
        cs: USER_CS as u64,
        ss: USER_DS as u64,
        ..CpuContext::empty()
    };

    // Tally allocated pages.
    let total_pages: u64 = image.segments.iter().map(|s| s.memsz / 4096).sum();
    proc.pages_allocated = total_pages;

    puts("[SCHED] spawned user PID ");
    put_hex32(pid);
    puts(" \"");
    puts(proc.name_str());
    puts("\" entry=");
    put_hex64(image.entry);
    puts(" cr3=");
    put_hex64(proc.cr3);
    puts("\n");

    PROCESS_TABLE[slot_idx] = Some(proc);
    LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
    Ok(pid)
}

// ═══════════════════════════════════════════════════════════════════════════
// BLOCKING PRIMITIVES (called from syscall handlers)
// ═══════════════════════════════════════════════════════════════════════════

/// Block the current process until a TSC deadline, then yield to the scheduler.
///
/// The process is marked `Blocked(Sleep(deadline))`.  The timer ISR will
/// eventually unblock it (via `wake_expired_sleepers`) and resume it.
///
/// # Safety
/// Must be called from a syscall handler with interrupts disabled.
pub unsafe fn block_sleep(deadline: u64) -> u64 {
    let pid = CURRENT_PID.load(Ordering::Relaxed) as usize;
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid) {
        proc.state = ProcessState::Blocked(BlockReason::Sleep(deadline));
    }

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
    let current = CURRENT_PID.load(Ordering::Relaxed);

    // Validate: does the child exist?
    let (child_parent, child_state) = match PROCESS_TABLE
        .get(child_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => (p.parent_pid, p.state),
        None => return u64::MAX - 3, // ESRCH
    };

    // Validate: is it actually our child?
    if child_parent != current {
        return u64::MAX - 3; // ESRCH
    }

    // Already reaped?
    if matches!(child_state, ProcessState::Terminated) {
        return u64::MAX - 10; // ECHILD
    }

    // Already zombie? Reap now.
    if child_state == ProcessState::Zombie {
        return reap_child(child_pid);
    }

    // Block until the child exits.
    let cur = current as usize;
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(cur) {
        proc.state = ProcessState::Blocked(BlockReason::WaitChild(child_pid));
    }

    // Yield — resume when terminate_process_inner unblocks us.
    core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));

    // Child should now be Zombie (terminate_process_inner set it).
    reap_child(child_pid)
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
/// Frees the kernel stack and records the deallocation.
/// User-space page tables and mapped physical frames are freed by walking
/// the PML4 hierarchy (only for non-kernel processes with cr3 ≠ kernel cr3).
unsafe fn free_process_resources(proc: &mut Process) {
    // ── Free kernel stack ────────────────────────────────────────────────
    if proc.kernel_stack_base != 0 && is_registry_initialized() {
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(PAGE_SIZE);
        let registry = global_registry_mut();
        let _ = registry.free_pages(proc.kernel_stack_base, pages);
        proc.kernel_stack_base = 0;
        proc.kernel_stack_top = 0;
    }

    // ── Free user page tables (if this isn't the kernel process) ─────────
    if proc.cr3 != 0 && proc.pid != 0 {
        // Read the kernel's CR3 so we don't accidentally free the kernel PML4.
        let kernel_cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) kernel_cr3, options(nostack, nomem));
        let kernel_cr3 = kernel_cr3 & 0x000F_FFFF_FFFF_F000;

        if proc.cr3 != kernel_cr3 {
            free_user_page_tables(proc.cr3, kernel_cr3);
            proc.cr3 = 0;
        }
    }
}

/// Walk a PML4 and free all non-kernel page table pages and physical frames.
///
/// We only free entries in the *lower half* of the address space (indices 0..255
/// of the PML4), which is the user region.  The upper half (256..511) belongs
/// to the kernel and is shared across all processes — we must not free those.
unsafe fn free_user_page_tables(pml4_phys: u64, _kernel_cr3: u64) {
    if !is_registry_initialized() {
        return;
    }
    let registry = global_registry_mut();

    let pml4 = pml4_phys as *const u64;

    // Walk PML4 entries 0..256 (user half).
    for pml4_idx in 0..256usize {
        let pml4e = *pml4.add(pml4_idx);
        if pml4e & 1 == 0 {
            continue;
        } // not present
        let pdpt_phys = pml4e & 0x000F_FFFF_FFFF_F000;
        let pdpt = pdpt_phys as *const u64;

        for pdpt_idx in 0..512usize {
            let pdpte = *pdpt.add(pdpt_idx);
            if pdpte & 1 == 0 {
                continue;
            }
            if pdpte & (1 << 7) != 0 {
                // 1 GiB huge page — free the physical frame.
                let frame = pdpte & 0x000F_FFFF_FFFF_F000;
                let _ = registry.free_pages(frame, 1 << 18); // 1 GiB = 262144 pages
                continue;
            }
            let pd_phys = pdpte & 0x000F_FFFF_FFFF_F000;
            let pd = pd_phys as *const u64;

            for pd_idx in 0..512usize {
                let pde = *pd.add(pd_idx);
                if pde & 1 == 0 {
                    continue;
                }
                if pde & (1 << 7) != 0 {
                    // 2 MiB huge page.
                    let frame = pde & 0x000F_FFFF_FFFF_F000;
                    let _ = registry.free_pages(frame, 512); // 2 MiB = 512 pages
                    continue;
                }
                let pt_phys = pde & 0x000F_FFFF_FFFF_F000;
                let pt = pt_phys as *const u64;

                for pt_idx in 0..512usize {
                    let pte = *pt.add(pt_idx);
                    if pte & 1 == 0 {
                        continue;
                    }
                    // 4 KiB page.
                    let frame = pte & 0x000F_FFFF_FFFF_F000;
                    let _ = registry.free_pages(frame, 1);
                }
                // Free the PT itself.
                let _ = registry.free_pages(pt_phys, 1);
            }
            // Free the PD itself.
            let _ = registry.free_pages(pd_phys, 1);
        }
        // Free the PDPT itself.
        let _ = registry.free_pages(pdpt_phys, 1);
    }
    // Free the PML4 itself.
    let _ = registry.free_pages(pml4_phys, 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Unblock any processes whose sleep deadline has been reached.
///
/// Called from `scheduler_tick` on every timer interrupt — must be fast.
unsafe fn wake_expired_sleepers() {
    let now = crate::cpu::tsc::read_tsc();
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if let ProcessState::Blocked(BlockReason::Sleep(deadline)) = proc.state {
            if now >= deadline {
                proc.state = ProcessState::Ready;
            }
        }
    }
}

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
