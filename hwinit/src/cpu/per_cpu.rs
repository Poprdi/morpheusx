//! Per-CPU data area.
//!
//! Each core gets one of these, accessed via the GS segment register.
//! `IA32_GS_BASE` (MSR 0xC0000101) points here in kernel mode.
//! `SWAPGS` swaps it with `IA32_KERNEL_GS_BASE` (MSR 0xC0000102) at
//! ring transitions so user code never sees our pointers.
//!
//! FIELD OFFSETS ARE ABI.  The ASM in context_switch.s and syscall.s
//! hardcodes `gs:[offset]` for the hot-path fields.  If you reorder
//! this struct, grep the asm/ directory and update every constant or
//! the next timer tick will be your last.

use crate::serial::{put_hex32, put_hex64, puts};
use core::sync::atomic::{AtomicU32, Ordering};

/// Maximum cores we support.  16 is generous for a desktop OS on QEMU.
/// Increase if you're running on a Threadripper and hate yourself.
pub const MAX_CPUS: usize = 16;

// ── ABI field offsets (must match struct layout below) ────────────────────
// These are used by context_switch.s and syscall.s via `gs:[OFFSET]`.
// Verified by `debug_assert_offsets()` at boot.

pub const PERCPU_SELF_PTR: usize = 0x00;
pub const PERCPU_CPU_ID: usize = 0x08;
pub const PERCPU_CURRENT_PID: usize = 0x0C;
pub const PERCPU_NEXT_CR3: usize = 0x10;
pub const PERCPU_FPU_PTR: usize = 0x18;
pub const PERCPU_KERNEL_RSP: usize = 0x20;
pub const PERCPU_USER_RSP_SCRATCH: usize = 0x28;
pub const PERCPU_TSS_PTR: usize = 0x30;
pub const PERCPU_LAPIC_BASE: usize = 0x38;
pub const PERCPU_TICK_COUNT: usize = 0x40;

/// Per-CPU data block.  One per core.  Accessed via GS segment.
///
/// # ABI contract
/// The first 0x48 bytes are read/written from ASM.  DO NOT reorder
/// fields without updating context_switch.s and syscall.s.
#[repr(C, align(64))] // cache-line aligned to avoid false sharing
pub struct PerCpu {
    // 0x00 — self pointer for sanity checks
    pub self_ptr: u64,
    // 0x08 — LAPIC ID (not sequential — comes from hardware)
    pub cpu_id: u32,
    // 0x0C — PID currently running on this core
    pub current_pid: u32,
    // 0x10 — CR3 to load on next context switch (set by scheduler_tick)
    pub next_cr3: u64,
    // 0x18 — pointer to current process's FpuState (for FXSAVE/FXRSTOR)
    pub current_fpu_ptr: u64,
    // 0x20 — kernel stack top for SYSCALL entry
    pub kernel_syscall_rsp: u64,
    // 0x28 — scratch slot for saving user RSP during SYSCALL
    pub user_rsp_scratch: u64,
    // 0x30 — pointer to this core's TSS (for updating RSP0)
    pub tss_ptr: u64,
    // 0x38 — LAPIC MMIO base (identity-mapped), typically 0xFEE0_0000
    pub lapic_base: u64,
    // 0x40 — per-core tick count
    pub tick_count: u64,
    // 0x48+ — less hot fields below, not accessed from ASM
    /// True while this core is inside scheduler_tick.
    pub in_tick: bool,
    /// True once this core has finished init and entered the scheduler.
    pub online: bool,
    /// AP's original kernel stack top (set during AP boot, never changed).
    /// Used to restore RSP when the AP returns to the idle loop after
    /// descheduling a user process.  BSP doesn't use this.
    pub boot_kernel_rsp: u64,
}

impl PerCpu {
    pub const fn zeroed() -> Self {
        Self {
            self_ptr: 0,
            cpu_id: 0,
            current_pid: 0,
            next_cr3: 0,
            current_fpu_ptr: 0,
            kernel_syscall_rsp: 0,
            user_rsp_scratch: 0,
            tss_ptr: 0,
            lapic_base: 0,
            tick_count: 0,
            in_tick: false,
            online: false,
            boot_kernel_rsp: 0,
        }
    }
}

// ── Global per-CPU array ─────────────────────────────────────────────────
// BSS-resident.  Index is the sequential core index (0 = BSP), NOT the
// LAPIC ID.  The LAPIC-ID-to-index mapping is in `LAPIC_TO_IDX`.

static mut PER_CPU_ARRAY: [PerCpu; MAX_CPUS] = [const { PerCpu::zeroed() }; MAX_CPUS];

/// Maps LAPIC ID → sequential core index.  Sparse (most entries 0xFF = unused).
static mut LAPIC_TO_IDX: [u8; 256] = [0xFF; 256];

/// Number of cores that have completed init.
pub static AP_ONLINE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Total number of detected CPUs (BSP + APs).  Set by BSP during MADT parse
/// or CPUID enumeration, before AP startup.
static mut CPU_COUNT: u32 = 1;

/// Set total detected CPU count.  Call once from BSP before starting APs.
pub unsafe fn set_cpu_count(n: u32) {
    CPU_COUNT = n;
}

/// Total detected CPUs (BSP + APs).
pub fn cpu_count() -> u32 {
    unsafe { CPU_COUNT }
}

// ── MSR helpers ──────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn wrmsr(msr: u32, val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, nomem),
    );
}

#[inline(always)]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, nomem),
    );
    (hi as u64) << 32 | lo as u64
}

const IA32_GS_BASE: u32 = 0xC000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

// ── Init ──────────────────────────────────────────────────────────────────

/// Initialize the BSP's PerCpu area and load GS-base.
///
/// Call once, after GDT is loaded, before enabling interrupts.
///
/// # Safety
/// Single-threaded BSP init.
pub unsafe fn init_bsp(lapic_id: u32, lapic_base: u64) {
    let idx = 0usize; // BSP is always core 0

    let pcpu = &mut PER_CPU_ARRAY[idx];
    pcpu.self_ptr = pcpu as *const PerCpu as u64;
    pcpu.cpu_id = lapic_id;
    pcpu.current_pid = 0; // kernel
    pcpu.lapic_base = lapic_base;
    pcpu.online = true;

    // map LAPIC ID → index
    LAPIC_TO_IDX[lapic_id as usize] = idx as u8;

    // load GS-base to point at this PerCpu
    let addr = pcpu as *const PerCpu as u64;
    wrmsr(IA32_GS_BASE, addr);
    // kernel GS-base starts as 0 — swapped in on transition to user mode
    wrmsr(IA32_KERNEL_GS_BASE, 0);

    AP_ONLINE_COUNT.store(1, Ordering::SeqCst);

    puts("[PERCPU] BSP core ");
    put_hex32(lapic_id);
    puts(" → idx 0, GS=");
    put_hex64(addr);
    puts("\n");

    debug_assert_offsets();
}

/// Initialize an AP's PerCpu area and load GS-base.
///
/// Called from the AP's Rust entry point after GDT/TSS are loaded.
///
/// # Safety
/// Called exactly once per AP, on the AP's own stack, interrupts disabled.
pub unsafe fn init_ap(core_idx: u32, lapic_id: u32, lapic_base: u64) {
    let idx = core_idx as usize;
    assert!(idx < MAX_CPUS);

    let pcpu = &mut PER_CPU_ARRAY[idx];
    pcpu.self_ptr = pcpu as *const PerCpu as u64;
    pcpu.cpu_id = lapic_id;
    pcpu.current_pid = 0; // starts idle on kernel
    pcpu.lapic_base = lapic_base;
    pcpu.online = true;

    // save the AP's kernel stack top so the scheduler can restore it
    // when the AP returns to idle after running a user process.
    let rsp: u64;
    core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem));
    pcpu.boot_kernel_rsp = (rsp + 0x1000) & !0xFFF;

    LAPIC_TO_IDX[lapic_id as usize] = idx as u8;

    let addr = pcpu as *const PerCpu as u64;
    wrmsr(IA32_GS_BASE, addr);
    wrmsr(IA32_KERNEL_GS_BASE, 0);

    AP_ONLINE_COUNT.fetch_add(1, Ordering::SeqCst);

    puts("[PERCPU] AP core ");
    put_hex32(lapic_id);
    puts(" → idx ");
    put_hex32(core_idx);
    puts(", GS=");
    put_hex64(addr);
    puts("\n");
}

// ── Accessors ────────────────────────────────────────────────────────────

/// Get the current core's PerCpu pointer via GS-base MSR.
/// Hot path — avoid this in the timer ISR.  ASM reads gs: directly.
#[inline(always)]
pub unsafe fn current() -> &'static mut PerCpu {
    let addr: u64;
    core::arch::asm!(
        "mov {}, gs:[0x00]", // self_ptr
        out(reg) addr,
        options(nostack, readonly, preserves_flags),
    );
    &mut *(addr as *mut PerCpu)
}

/// Get PerCpu by sequential core index.
#[inline(always)]
pub unsafe fn by_index(idx: u32) -> &'static mut PerCpu {
    &mut PER_CPU_ARRAY[idx as usize]
}

/// Get PerCpu by LAPIC ID.
#[inline(always)]
pub unsafe fn by_lapic_id(lapic_id: u32) -> Option<&'static mut PerCpu> {
    let idx = LAPIC_TO_IDX[lapic_id as usize];
    if idx == 0xFF {
        None
    } else {
        Some(&mut PER_CPU_ARRAY[idx as usize])
    }
}

/// Current core's sequential index (0 = BSP).
#[inline(always)]
pub unsafe fn current_core_index() -> u32 {
    let lapic_id: u32;
    core::arch::asm!(
        "mov {0:e}, gs:[0x08]", // cpu_id is u32 at offset 0x08. read 32-bit to avoid
                                 // bleeding into current_pid at 0x0C.
        out(reg) lapic_id,
        options(nostack, readonly, preserves_flags),
    );
    LAPIC_TO_IDX[lapic_id as usize] as u32
}

/// Current core's LAPIC ID.
#[inline(always)]
pub unsafe fn current_lapic_id() -> u32 {
    let id: u32;
    core::arch::asm!(
        "mov {0:e}, gs:[0x08]",
        out(reg) id,
        options(nostack, readonly, preserves_flags),
    );
    id
}

/// PID running on the current core.
#[inline(always)]
pub unsafe fn current_pid() -> u32 {
    let pid: u32;
    core::arch::asm!(
        "mov {0:e}, gs:[0x0C]",
        out(reg) pid,
        options(nostack, readonly, preserves_flags),
    );
    pid
}

/// Set the PID running on the current core.
#[inline(always)]
pub unsafe fn set_current_pid(pid: u32) {
    core::arch::asm!(
        "mov gs:[0x0C], {0:e}",
        in(reg) pid,
        options(nostack, preserves_flags),
    );
}

/// Check if SMP is active (more than 1 core online).
#[inline(always)]
pub fn smp_active() -> bool {
    AP_ONLINE_COUNT.load(Ordering::Relaxed) > 1
}

// ── Offset validation ────────────────────────────────────────────────────

/// Compile-time offset checks would be ideal, but we can't use
/// `offset_of!` in const context on stable.  Runtime assert at BSP
/// init is the next best thing — fires exactly once, before any
/// AP or interrupt could use the wrong offsets.
fn debug_assert_offsets() {
    let base = core::ptr::null::<PerCpu>();
    unsafe {
        let check = |field_ptr: *const u8, expected: usize, name: &str| {
            let actual = field_ptr as usize - base as usize;
            if actual != expected {
                puts("[PERCPU] FATAL: ");
                puts(name);
                puts(" at ");
                put_hex32(actual as u32);
                puts(" expected ");
                put_hex32(expected as u32);
                puts("\n");
                panic!("PerCpu layout mismatch");
            }
        };
        check(
            core::ptr::addr_of!((*base).self_ptr) as *const u8,
            PERCPU_SELF_PTR,
            "self_ptr",
        );
        check(
            core::ptr::addr_of!((*base).cpu_id) as *const u8,
            PERCPU_CPU_ID,
            "cpu_id",
        );
        check(
            core::ptr::addr_of!((*base).current_pid) as *const u8,
            PERCPU_CURRENT_PID,
            "current_pid",
        );
        check(
            core::ptr::addr_of!((*base).next_cr3) as *const u8,
            PERCPU_NEXT_CR3,
            "next_cr3",
        );
        check(
            core::ptr::addr_of!((*base).current_fpu_ptr) as *const u8,
            PERCPU_FPU_PTR,
            "current_fpu_ptr",
        );
        check(
            core::ptr::addr_of!((*base).kernel_syscall_rsp) as *const u8,
            PERCPU_KERNEL_RSP,
            "kernel_syscall_rsp",
        );
        check(
            core::ptr::addr_of!((*base).user_rsp_scratch) as *const u8,
            PERCPU_USER_RSP_SCRATCH,
            "user_rsp_scratch",
        );
        check(
            core::ptr::addr_of!((*base).tss_ptr) as *const u8,
            PERCPU_TSS_PTR,
            "tss_ptr",
        );
        check(
            core::ptr::addr_of!((*base).lapic_base) as *const u8,
            PERCPU_LAPIC_BASE,
            "lapic_base",
        );
        check(
            core::ptr::addr_of!((*base).tick_count) as *const u8,
            PERCPU_TICK_COUNT,
            "tick_count",
        );
    }
}
