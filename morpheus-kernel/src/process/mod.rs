//! Process table + round-robin scheduler. PID 0 = kernel, always runnable.
//! Scheduler enters from the timer ISR. `PROCESS_TABLE` is a fixed static array;
//! no heap allocation on the scheduling path.

pub mod signals;
pub mod vma;

pub use morpheus_hal_api::{CpuContext, FpuState};
pub use signals::{Signal, SignalAction, SignalSet};
pub use vma::{Vma, VmaTable};

use crate::hal;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use morpheus_hal_api::{AllocKind, MemoryType};

/// Total task slots: processes + threads share one id-indexed table (slot = pid/tid).
/// Raised from 64 so threads draw from a budget separate from the independent-process
/// cap (`MAX_USER_PROCESSES`) and can't starve `fork`/`spawn` of a slot.
pub const MAX_PROCESSES: usize = 256;

/// Cap on live independent processes (thread-group leaders); threads aren't counted.
pub const MAX_USER_PROCESSES: usize = 64;

/// 64-bit words needed to hold one bit per task slot.
pub const PIDSET_WORDS: usize = MAX_PROCESSES.div_ceil(64);

/// Lock-free set of task slots, one bit per pid; scales waiter tracking past 64 slots.
pub struct PidBitset {
    words: [AtomicU64; PIDSET_WORDS],
}

impl PidBitset {
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU64::new(0) }; PIDSET_WORDS],
        }
    }

    #[inline(always)]
    fn locate(pid: u32) -> (usize, u64) {
        let p = pid as usize;
        (p / 64, 1u64 << (p % 64))
    }

    #[inline]
    pub fn set(&self, pid: u32) {
        if (pid as usize) >= MAX_PROCESSES {
            return;
        }
        let (w, b) = Self::locate(pid);
        self.words[w].fetch_or(b, Ordering::Relaxed);
    }

    #[inline]
    pub fn clear(&self, pid: u32) {
        if (pid as usize) >= MAX_PROCESSES {
            return;
        }
        let (w, b) = Self::locate(pid);
        self.words[w].fetch_and(!b, Ordering::Relaxed);
    }

    #[inline]
    pub fn contains(&self, pid: u32) -> bool {
        if (pid as usize) >= MAX_PROCESSES {
            return false;
        }
        let (w, b) = Self::locate(pid);
        self.words[w].load(Ordering::Relaxed) & b != 0
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| w.load(Ordering::Relaxed) == 0)
    }

    /// Invoke `f` for every set pid. Per-word snapshot; sets racing the load show up next call.
    #[inline]
    pub fn for_each(&self, mut f: impl FnMut(u32)) {
        for (wi, word) in self.words.iter().enumerate() {
            let mut m = word.load(Ordering::Relaxed);
            while m != 0 {
                let b = m.trailing_zeros();
                m &= m - 1;
                f((wi as u32) * 64 + b);
            }
        }
    }
}

impl Default for PidBitset {
    fn default() -> Self {
        Self::new()
    }
}

/// FIONBIO state, one bit per PID. Kept out of `Process` so the struct's ABI
/// layout stays byte-identical — `Process` feeds fixed-offset asm
/// (context_switch.s / syscall.s) and array-strided FXSAVE areas, so growing the
/// HOT prefix is a real-hardware footgun. fd 0 has no descriptor to hang a flag
/// on, hence a side table. Bit set ⇒ `SYS_READ(0)` returns EAGAIN instead of
/// blocking on an empty stdin.
static STDIN_NONBLOCK: PidBitset = PidBitset::new();

/// True if `pid` requested non-blocking stdin. Out-of-range PIDs read false.
pub fn stdin_nonblock(pid: u32) -> bool {
    STDIN_NONBLOCK.contains(pid)
}

/// Set/clear `pid`'s non-blocking stdin bit. No-op for out-of-range PIDs.
pub fn set_stdin_nonblock(pid: u32, enable: bool) {
    if enable {
        STDIN_NONBLOCK.set(pid);
    } else {
        STDIN_NONBLOCK.clear(pid);
    }
}

pub const PROCESS_KERNEL_STACK_SIZE: usize = 128 * 1024;

pub const SCHED_CAP_CAN_MIGRATE: u32 = 1 << 0;
pub const SCHED_CAP_CAN_PARK: u32 = 1 << 1;
pub const SCHED_CAP_HAS_RT_HINT: u32 = 1 << 2;
pub const SCHED_CAP_PINNED: u32 = 1 << 3;
pub const SCHED_CAP_DEFAULT: u32 =
    SCHED_CAP_CAN_MIGRATE | SCHED_CAP_CAN_PARK | SCHED_CAP_HAS_RT_HINT;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessPowerMode {
    Performance = 0,
    Balanced = 1,
    Eco = 2,
    ThermalClamp = 3,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessPolicyClass {
    LatencyCritical = 0,
    Interactive = 1,
    Throughput = 2,
    Background = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// TSC-tick deadline.
    Sleep(u64),
    WaitChild(u32),
    /// Driver unblocks externally.
    Io,
    StdinRead,
    /// `PIPE_TABLE` index.
    PipeRead(u8),
    /// Compositor-forwarded keyboard input.
    InputRead,
    /// Futex word at this user VA.
    FutexWait(u64),
    /// Parked on an I/O readiness token (socket/pipe/epoll); `futex_deadline` is
    /// reused to arm an optional timeout.
    IoReady(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked(BlockReason),
    /// Exited; parent must reap via waitpid().
    Zombie,
    /// Reaped; slot is free.
    Terminated,
}

impl ProcessState {
    pub fn is_runnable(self) -> bool {
        matches!(self, ProcessState::Ready | ProcessState::Running)
    }
}

/// Kernel process descriptor. Cache-aligned (64 B) against SMP false-sharing of hot fields.
#[repr(C, align(64))]
pub struct Process {
    pub pid: u32,
    /// UTF-8, NUL-terminated.
    pub name: [u8; 32],
    /// 0 = no parent.
    pub parent_pid: u32,

    pub state: ProcessState,
    pub exit_code: Option<i32>,

    /// PML4 phys for this process; kernel threads share the kernel PML4.
    pub cr3: u64,
    pub kernel_stack_top: u64,
    /// Base of the kernel stack allocation (for free).
    pub kernel_stack_base: u64,
    /// Saved registers; populated on every context switch out.
    pub context: CpuContext,

    /// FXSAVE area; saved/restored by the timer ISR on every switch.
    pub fpu_state: FpuState,
    /// Restored by SYS_SIGRETURN.
    pub saved_signal_fpu: FpuState,

    /// `(base, size_bytes)`.
    pub heap_region: (u64, u64),
    pub pages_allocated: u64,

    /// Lower = higher; 0 = real-time.
    pub priority: u8,
    /// Remaining weighted quanta in the current RR epoch.
    pub sched_budget_left: u8,
    /// Ready-but-not-selected ticks; drives starvation forcing.
    pub sched_wait_ticks: u32,
    pub cpu_ticks: u64,
    /// TSC cycles *actively* running (excludes HLT).
    pub cpu_tsc: u64,
    /// TSC at start of current quantum.
    pub run_start_tsc: u64,
    /// Exokernel importance hint (1..=16).
    pub importance_16: u8,
    pub power_mode: ProcessPowerMode,
    pub policy_class: ProcessPolicyClass,
    /// Bit i = runnable preference on core i.
    pub affinity_mask: u64,
    pub policy_flags: u32,
    pub capability_bits: u32,
    /// Effective weight after clamping/policy.
    pub effective_weight_cache: u8,

    pub pending_signals: signals::SignalSet,
    /// 0 = SIG_DFL, 1 = SIG_IGN, >1 = user fn.
    pub signal_handlers: [u64; 32],

    pub fd_table: crate::storage::fs_api::FdTable,

    /// Bump pointer for SYS_MMAP.
    pub mmap_brk: u64,
    /// Tracks mmap'd regions for proper munmap.
    pub vma_table: VmaTable,

    /// NUL-terminated, max 255 chars.
    pub cwd: [u8; 256],
    pub cwd_len: u16,

    /// Spawn args, NUL-separated; retrieved via SYS_GETARGS.
    pub args: [u8; 256],
    pub args_len: u16,
    pub argc: u8,

    /// 0 = independent. Nonzero = leader PID whose CR3 this thread shares.
    pub thread_group_leader: u32,

    /// TSC deadline; 0 = forever.
    pub futex_deadline: u64,

    /// Restored by SYS_SIGRETURN.
    pub saved_signal_context: CpuContext,
    /// Blocks nested signal delivery.
    pub in_signal_handler: bool,

    pub fb_surface_phys: u64,
    pub fb_surface_pages: u64,
    pub fb_surface_dirty: bool,

    pub mouse_dx: i32,
    pub mouse_dy: i32,
    pub mouse_buttons: u8,

    pub input_buf: [u8; 256],
    pub input_head: u8,
    pub input_tail: u8,

    pub running_on: u32,

    /// x86 FS base; restored on every switch-to-user. Layout-safe: no fixed-offset asm reads it.
    pub tls_base: u64,

    /// CLONE_CHILD_CLEARTID address: on this thread's exit the kernel writes 0
    /// here and FUTEX_WAKEs it, so a join from ANY sibling is race-free. 0 = none.
    pub ctid_ptr: u64,
    /// Thread auto-reaps on exit (no zombie); set via THREAD_DETACHED or SYS_THREAD_DETACH.
    pub detached: bool,
    /// Signal that terminated this task (0 = normal exit); drives WIFSIGNALED.
    pub term_signal: u8,
    /// Set by the tick when a `FUTEX_WAIT` deadline expires, so the resuming
    /// `sys_futex` returns `-ETIMEDOUT` rather than 0. Read-and-cleared there.
    pub futex_timed_out: bool,
    /// Initial environment block: NUL-separated `KEY=VALUE` records backing
    /// SYS_GETENV / std::env. Heap-backed so it is not part of the hot asm prefix.
    pub env_block: Vec<u8>,
}

impl Process {
    /// Const-init empty slot. FPU state zeroed; HAL `fpu_init` runs lazily on first spawn
    /// (avoiding `hal()` in const context).
    pub const fn empty() -> Self {
        let mut cwd = [0u8; 256];
        cwd[0] = b'/';
        Self {
            pid: 0,
            name: [0u8; 32],
            parent_pid: 0,
            state: ProcessState::Terminated,
            exit_code: None,
            cr3: 0,
            kernel_stack_top: 0,
            kernel_stack_base: 0,
            context: CpuContext::zeroed(),
            fpu_state: FpuState::zeroed(),
            saved_signal_fpu: FpuState::zeroed(),
            heap_region: (0, 0),
            pages_allocated: 0,
            priority: 128,
            sched_budget_left: 0,
            sched_wait_ticks: 0,
            cpu_ticks: 0,
            cpu_tsc: 0,
            run_start_tsc: 0,
            importance_16: 8,
            power_mode: ProcessPowerMode::Balanced,
            policy_class: ProcessPolicyClass::Throughput,
            affinity_mask: u64::MAX,
            policy_flags: 0,
            capability_bits: SCHED_CAP_DEFAULT,
            effective_weight_cache: 0,
            pending_signals: signals::SignalSet::empty(),
            signal_handlers: [0u64; 32],
            fd_table: crate::storage::fs_api::FdTable::new(),
            mmap_brk: 0,
            vma_table: VmaTable::new(),
            cwd,
            cwd_len: 1,
            args: [0u8; 256],
            args_len: 0,
            argc: 0,
            thread_group_leader: 0,
            futex_deadline: 0,
            saved_signal_context: CpuContext::zeroed(),
            in_signal_handler: false,
            fb_surface_phys: 0,
            fb_surface_pages: 0,
            fb_surface_dirty: false,
            mouse_dx: 0,
            mouse_dy: 0,
            mouse_buttons: 0,
            input_buf: [0u8; 256],
            input_head: 0,
            input_tail: 0,
            running_on: u32::MAX,
            tls_base: 0,
            ctid_ptr: 0,
            detached: false,
            term_signal: 0,
            futex_timed_out: false,
            env_block: Vec::new(),
        }
    }

    /// True iff this slot is a thread (shares a leader's CR3), not an independent process.
    pub fn is_thread(&self) -> bool {
        self.thread_group_leader != 0
    }

    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }

    pub fn set_cwd(&mut self, path: &str) {
        let bytes = path.as_bytes();
        let len = bytes.len().min(255);
        self.cwd[..len].copy_from_slice(&bytes[..len]);
        self.cwd[len] = 0;
        self.cwd_len = len as u16;
    }

    pub fn cwd_str(&self) -> &str {
        let len = self.cwd_len as usize;
        core::str::from_utf8(&self.cwd[..len]).unwrap_or("/")
    }

    /// Trimmed at first NUL.
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("<?>")
    }

    pub fn is_free(&self) -> bool {
        matches!(self.state, ProcessState::Terminated)
    }

    /// Fills `kernel_stack_base`/`top` and seeds FPU defaults (lazy counterpart to
    /// `empty()` zeroing the blob in const context).
    ///
    /// # Safety
    /// HAL must be installed.
    pub unsafe fn alloc_kernel_stack(&mut self) -> Result<(), &'static str> {
        let phys = hal().phys();
        if !phys.is_initialized() {
            return Err("PhysAlloc not ready");
        }
        let pages = (PROCESS_KERNEL_STACK_SIZE as u64).div_ceil(phys.page_size());
        let base = phys
            .allocate_pages(AllocKind::AnyPages, MemoryType::AllocatedStack, pages)
            .map_err(|_| "failed to allocate kernel stack")?;
        self.kernel_stack_base = base;
        self.kernel_stack_top = base + PROCESS_KERNEL_STACK_SIZE as u64;
        self.pages_allocated += pages;
        hal().cpu().fpu_init(&mut self.fpu_state);
        hal().cpu().fpu_init(&mut self.saved_signal_fpu);
        Ok(())
    }
}
