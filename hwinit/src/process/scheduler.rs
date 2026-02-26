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
// GLOBAL STATE

/// The flat process table.  Index == PID.
pub(crate) static mut PROCESS_TABLE: [Option<Process>; MAX_PROCESSES] = {
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

// PUBLIC INFO SNAPSHOT (allocation-free, for the task manager)

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

impl ProcessInfo {
    /// Create a zeroed ProcessInfo (used to pre-fill arrays).
    pub const fn zeroed() -> Self {
        Self {
            pid: 0,
            name: [0u8; 32],
            state: ProcessState::Ready,
            cpu_ticks: 0,
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

impl Scheduler {
    /// Snapshot the process table for display (e.g. task manager).
    ///
    /// Fills `out` with up to `out.len()` entries and returns how many were
    /// written.  No allocation.
    /// This broke me, fuck this schedular but somehow it "works"
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

    /// Mutable reference to the current process descriptor.
    ///
    /// # Safety
    /// Single-threaded; call only with interrupts disabled (e.g. from a syscall handler).
    pub unsafe fn current_process_mut(&self) -> &'static mut Process {
        let pid = CURRENT_PID.load(Ordering::Relaxed) as usize;
        PROCESS_TABLE[pid].as_mut().unwrap()
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
        let slot = PROCESS_TABLE
            .get_mut(pid as usize)
            .and_then(|s| s.as_mut())
            .ok_or("send_signal: PID not found")?;

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

    /// Set the scheduling priority of a process.
    pub unsafe fn set_priority(&self, pid: u32, priority: u8) -> Result<(), &'static str> {
        let slot = PROCESS_TABLE
            .get_mut(pid as usize)
            .and_then(|s| s.as_mut())
            .ok_or("set_priority: PID not found")?;
        if slot.is_free() {
            return Err("set_priority: process terminated");
        }
        slot.priority = priority;
        Ok(())
    }

    /// Get the scheduling priority of a process.
    pub unsafe fn get_priority(&self, pid: u32) -> Result<u8, &'static str> {
        let slot = PROCESS_TABLE
            .get(pid as usize)
            .and_then(|s| s.as_ref())
            .ok_or("get_priority: PID not found")?;
        if slot.is_free() {
            return Err("get_priority: process terminated");
        }
        Ok(slot.priority)
    }
}

/// Initialize the scheduler and create PID 0 (the kernel process).
///
/// Must be called once, after MemoryRegistry, GDT, IDT, and heap are ready.
///
/// # Safety
/// Single-threaded init; not reentrant(for now).
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

/// Terminate the calling process with the given exit code.
///
/// Transitions the current process to `Zombie` and yields to the scheduler.
/// The scheduler will pick another ready process.
///
/// # Safety
/// Must be called from within a process context (not the timer ISR itself xD).
pub unsafe fn exit_process(code: i32) -> ! {
    let pid = CURRENT_PID.load(Ordering::Relaxed);

    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        terminate_process_inner(proc, code);
    }
    // Re-enable interrupts so the timer ISR can fire and the scheduler
    // switches away.  We are Zombie — the scheduler will never pick us again.
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

    // Pick next runnable process (round-robin, priority-weighted in "future").
    // TODO: implement priority-weighted scheduling instead of pure round-robin.
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

// USER PROCESS SPAWN

/// Spawn a Ring 3 user thread in the caller's address space.
///
/// Shares the parent's CR3 (same page tables, same heap, same mmap state).
/// Gets its own kernel stack, its own slot in PROCESS_TABLE, and starts
/// execution at `entry` with `rdi = arg` and `rsp = stack_top`.
///
/// The caller must provide a valid, mapped user stack.  Use SYS_MMAP to
/// allocate one before calling this.
///
/// I had fun with this.
///
/// Returns the new thread's PID (which also serves as the TID).
pub unsafe fn spawn_user_thread(entry: u64, stack_top: u64, arg: u64) -> Result<u32, &'static str> {
    use crate::cpu::gdt::{USER_CS, USER_DS};

    if !SCHEDULER_READY {
        return Err("scheduler not initialized");
    }

    let parent_pid = CURRENT_PID.load(Ordering::Relaxed);
    let parent = match PROCESS_TABLE
        .get(parent_pid as usize)
        .and_then(|s| s.as_ref())
    {
        Some(p) => p,
        None => return Err("no current process"),
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
        .ok_or("process table full")?;

    let tid = slot_idx as u32;

    PROCESS_TABLE[slot_idx] = Some(Process::empty());
    let thread = PROCESS_TABLE[slot_idx]
        .as_mut()
        .ok_or("failed to initialize thread slot")?;

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

    // fd inheritance
    if inherit_fds {
        let parent_pid = proc.parent_pid as usize;
        if let Some(Some(parent)) = PROCESS_TABLE.get(parent_pid) {
            proc.fd_table = parent.fd_table;
            // Increment pipe refcounts for inherited pipe fds.
            use morpheus_helix::types::open_flags;
            for fd_desc in proc.fd_table.fds.iter() {
                if fd_desc.is_open() {
                    let fl = fd_desc.flags;
                    if fl & open_flags::O_PIPE_READ != 0 {
                        crate::pipe::pipe_add_reader(fd_desc.mount_idx);
                    } else if fl & open_flags::O_PIPE_WRITE != 0 {
                        crate::pipe::pipe_add_writer(fd_desc.mount_idx);
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

// BLOCKING PRIMITIVES (called from syscall handlers)

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
    // free kernel stack
    if proc.kernel_stack_base != 0 && is_registry_initialized() {
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(PAGE_SIZE);
        let registry = global_registry_mut();
        let _ = registry.free_pages(proc.kernel_stack_base, pages);
        proc.kernel_stack_base = 0;
        proc.kernel_stack_top = 0;
    }

    // free user page tables (if this isn't the kernel process and not a thread)
    // Threads share their leader's CR3 — freeing it would nuke the parent.
    if proc.cr3 != 0 && proc.pid != 0 && proc.thread_group_leader == 0 {
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

// HELPERS

/// Unblock any processes whose sleep deadline has been reached.
///
/// Called from `scheduler_tick` on every timer interrupt — must be fast.
unsafe fn wake_expired_sleepers() {
    let now = crate::cpu::tsc::read_tsc();
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        match proc.state {
            ProcessState::Blocked(BlockReason::Sleep(deadline)) => {
                if now >= deadline {
                    proc.state = ProcessState::Ready;
                }
            }
            ProcessState::Blocked(BlockReason::FutexWait(_)) => {
                if proc.futex_deadline != 0 && now >= proc.futex_deadline {
                    proc.state = ProcessState::Ready;
                    proc.futex_deadline = 0;
                }
            }
            _ => {}
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
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if matches!(proc.state, ProcessState::Blocked(BlockReason::StdinRead)) {
            proc.state = ProcessState::Ready;
        }
    }
}

/// Wake all processes blocked on `BlockReason::PipeRead(idx)`.
///
/// Called after writing to a pipe so that blocked readers can proceed.
pub unsafe fn wake_pipe_readers(pipe_idx: u8) {
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if let ProcessState::Blocked(BlockReason::PipeRead(idx)) = proc.state {
            if idx == pipe_idx {
                proc.state = ProcessState::Ready;
            }
        }
    }
}

/// Wake up to `count` processes blocked on `FutexWait(addr)`.
///
/// Returns number actually woken.  If count == u32::MAX, wakes all.
pub unsafe fn wake_futex_waiters(addr: u64, count: u32) -> u32 {
    let mut woken = 0u32;
    for proc in PROCESS_TABLE.iter_mut().flatten() {
        if woken >= count {
            break;
        }
        if let ProcessState::Blocked(BlockReason::FutexWait(wait_addr)) = proc.state {
            if wait_addr == addr {
                proc.state = ProcessState::Ready;
                woken += 1;
            }
        }
    }
    woken
}
