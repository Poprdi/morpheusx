//! Process subsystem — process table, state machine, and round-robin scheduler.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────┐
//! │                       PROCESS TABLE                               │
//! │  [0] = kernel (PID 0, always Running or Ready, never killed)      │
//! │  [1..MAX_PROCESSES-1] = user/app processes                        │
//! └────────────────────────────────────────────────────────────────────┘
//!               │
//!               ▼
//! ┌────────────────────────────────────────────────────────────────────┐
//! │                       SCHEDULER                                   │
//! │  Round-robin over Ready processes.                                │
//! │  Timer ISR calls scheduler_tick() → context switch if due.        │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Thread safety
//!
//! The system is currently single-core.  The scheduler is entered only from
//! the timer ISR (which is non-reentrant by design — PIC masks that IRQ during
//! the handler).  `spin::Mutex` wrappers are present for correctness; actual
//! SMP support would require IPI broadcasts and per-CPU run queues.
//!
//! # No `alloc` in the critical path
//!
//! `PROCESS_TABLE` is a fixed static array.  No heap allocation is performed
//! during scheduling.  Heap is only used when first creating a process
//! (to allocate its kernel stack via MemoryRegistry, not via Vec/Box).

pub mod context;
pub mod scheduler;
pub mod signals;

pub use context::CpuContext;
pub use scheduler::{
    Scheduler, ProcessInfo,
    SCHEDULER,
    init_scheduler,
    spawn_kernel_thread,
    exit_process,
    scheduler_tick,
};
pub use signals::{Signal, SignalSet};

use crate::memory::{AllocateType, MemoryType, PAGE_SIZE, global_registry_mut, is_registry_initialized};
use crate::serial::puts;

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of concurrent processes (including PID 0 kernel).
pub const MAX_PROCESSES: usize = 64;

/// Per-process kernel stack size.
pub const PROCESS_KERNEL_STACK_SIZE: usize = 32 * 1024; // 32 KiB

// ═══════════════════════════════════════════════════════════════════════════
// PROCESS STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Reason a process is blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// Sleeping until a TSC-tick deadline.
    Sleep(u64),
    /// Waiting for a child process to exit.
    WaitChild(u32),
    /// Waiting for I/O (unblocked externally by a driver).
    Io,
}

/// Process lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    /// Ready to run; waiting for the scheduler.
    Ready,
    /// Currently on-CPU.
    Running,
    /// Blocked — waiting for an event.
    Blocked(BlockReason),
    /// Exited; exit_code set.  Parent must reap via waitpid().
    Zombie,
    /// Reaped (slot is free).
    Terminated,
}

impl ProcessState {
    pub fn is_runnable(self) -> bool {
        matches!(self, ProcessState::Ready | ProcessState::Running)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROCESS
// ═══════════════════════════════════════════════════════════════════════════

/// A kernel process descriptor.
///
/// Stored in a fixed-size static slot; no heap allocation for the descriptor
/// itself.  The kernel stack is allocated from MemoryRegistry.
pub struct Process {
    // ── Identity ─────────────────────────────────────────────────────────
    pub pid:        u32,
    /// Process name — UTF-8, NUL-terminated, stored inline.
    pub name:       [u8; 32],
    pub parent_pid: u32,     // 0 = no parent

    // ── State ─────────────────────────────────────────────────────────────
    pub state:      ProcessState,
    pub exit_code:  Option<i32>,

    // ── CPU ───────────────────────────────────────────────────────────────
    /// Physical address of this process's PML4 table.
    /// For kernel threads this is identical to the kernel PML4 (shared).
    pub cr3:        u64,
    /// Top of this process's kernel stack (for interrupt/syscall entry).
    pub kernel_stack_top: u64,
    /// Physical base address of the kernel stack allocation (to free later).
    pub kernel_stack_base: u64,
    /// Saved register state (populated on every context switch away).
    pub context:    CpuContext,

    // ── Memory ────────────────────────────────────────────────────────────
    /// Virtual address range of the user heap: `(base, size_bytes)`.
    pub heap_region: (u64, u64),
    /// Total number of 4 KiB pages allocated for this process.
    pub pages_allocated: u64,

    // ── Scheduling ────────────────────────────────────────────────────────
    /// Scheduling priority (lower = higher priority; 0 = real-time).
    pub priority:   u8,
    /// Accumulated CPU ticks (for the task manager display).
    pub cpu_ticks:  u64,

    // ── Signals ───────────────────────────────────────────────────────────
    pub pending_signals: signals::SignalSet,

    // ── File descriptors ─────────────────────────────────────────────────
    /// Per-process file descriptor table.
    pub fd_table: morpheus_helix::vfs::FdTable,
}

impl Process {
    pub const fn empty() -> Self {
        Self {
            pid:              0,
            name:             [0u8; 32],
            parent_pid:       0,
            state:            ProcessState::Terminated,
            exit_code:        None,
            cr3:              0,
            kernel_stack_top:  0,
            kernel_stack_base: 0,
            context:          CpuContext::empty(),
            heap_region:      (0, 0),
            pages_allocated:  0,
            priority:         128,
            cpu_ticks:        0,
            pending_signals:  signals::SignalSet::empty(),
            fd_table:         morpheus_helix::vfs::FdTable::new(),
        }
    }

    /// Write a name string into the inline name buffer.
    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }

    /// Return the process name as a `&str` (trimmed at the first NUL).
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("<?>")
    }

    /// True if the slot is not in use.
    pub fn is_free(&self) -> bool {
        matches!(self.state, ProcessState::Terminated)
    }

    /// Allocate a kernel stack for this process from MemoryRegistry.
    ///
    /// Fills `kernel_stack_base` and `kernel_stack_top`.
    ///
    /// # Safety
    /// MemoryRegistry must be initialized.
    pub unsafe fn alloc_kernel_stack(&mut self) -> Result<(), &'static str> {
        if !is_registry_initialized() {
            return Err("MemoryRegistry not ready");
        }
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let registry = global_registry_mut();
        let base = registry
            .allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedStack, pages)
            .map_err(|_| "failed to allocate kernel stack")?;
        self.kernel_stack_base = base;
        self.kernel_stack_top  = base + PROCESS_KERNEL_STACK_SIZE as u64;
        self.pages_allocated  += pages;
        Ok(())
    }
}
