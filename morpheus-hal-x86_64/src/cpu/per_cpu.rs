//! Per-CPU area via GS. `IA32_GS_BASE` (MSR 0xC0000101) points here in ring 0;
//! `SWAPGS` exchanges with `IA32_KERNEL_GS_BASE` (MSR 0xC0000102) at ring transitions.
//!
//! FIELD OFFSETS ARE ABI. context_switch.s and syscall.s hardcode `gs:[offset]`.
//! Reordering without updating the asm + constants below = next tick is your last.

use crate::serial::{put_hex32, puts};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub const MAX_CPUS: usize = 16;
const LAPIC_ID_MAP_SIZE: usize = 1024;

// ABI offsets used by asm; `debug_assert_offsets()` runs at BSP init.

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

/// # ABI contract
/// First 0x48 bytes are read/written from ASM. Reorder = update context_switch.s and syscall.s.
#[repr(C, align(64))]
pub struct PerCpu {
    /// 0x00 — sanity self-pointer.
    pub self_ptr: u64,
    /// 0x08 — LAPIC ID (hardware, not sequential).
    pub cpu_id: u32,
    /// 0x0C — currently-running PID.
    pub current_pid: u32,
    /// 0x10 — CR3 for next context switch (set by scheduler_tick).
    pub next_cr3: u64,
    /// 0x18 — current process's FpuState (FXSAVE/FXRSTOR target).
    pub current_fpu_ptr: u64,
    /// 0x20 — SYSCALL entry kernel stack.
    pub kernel_syscall_rsp: u64,
    /// 0x28 — SYSCALL user-RSP save slot.
    pub user_rsp_scratch: u64,
    /// 0x30 — this core's TSS (for RSP0 updates).
    pub tss_ptr: u64,
    /// 0x38 — LAPIC MMIO base (identity-mapped, typ. 0xFEE0_0000).
    pub lapic_base: u64,
    /// 0x40 — per-core tick count.
    pub tick_count: u64,
    // 0x48+ : not touched by ASM.
    pub in_tick: bool,
    pub online: bool,
    /// AP's original kernel stack top — restored by scheduler when AP returns
    /// to idle after descheduling a user proc. Unused on BSP.
    pub boot_kernel_rsp: u64,
    pub load_hint: u32,
    pub park_state: u8,
    pub last_active_tsc: u64,
    /// Sequential index (0 = BSP); cached so `current_core_index()` skips the LAPIC-ID lookup.
    pub core_index: u32,
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
            load_hint: 0,
            park_state: 0,
            last_active_tsc: 0,
            core_index: 0,
        }
    }
}

// BSS array, indexed by sequential core index (0 = BSP), not LAPIC ID.
static mut PER_CPU_ARRAY: [PerCpu; MAX_CPUS] = [const { PerCpu::zeroed() }; MAX_CPUS];

/// LAPIC ID → core index. Sparse; 0xFF = unused.
static mut LAPIC_TO_IDX: [u8; LAPIC_ID_MAP_SIZE] = [0xFF; LAPIC_ID_MAP_SIZE];

#[inline(always)]
unsafe fn map_lapic_id(lapic_id: u32, idx: u8) {
    if (lapic_id as usize) < LAPIC_ID_MAP_SIZE {
        LAPIC_TO_IDX[lapic_id as usize] = idx;
    } else {
        puts("[PERCPU] LAPIC ID out of fast-map range: ");
        put_hex32(lapic_id);
        puts("\n");
    }
}

#[inline(always)]
unsafe fn lapic_id_to_index(lapic_id: u32) -> Option<u32> {
    if (lapic_id as usize) < LAPIC_ID_MAP_SIZE {
        let idx = LAPIC_TO_IDX[lapic_id as usize];
        if idx != 0xFF {
            return Some(idx as u32);
        }
    }

    // Fallback scan for sparse/high IDs; MAX_CPUS tiny.
    #[allow(clippy::needless_range_loop)]
    for i in 0..MAX_CPUS {
        let pcpu = &PER_CPU_ARRAY[i];
        if pcpu.online && pcpu.cpu_id == lapic_id {
            return Some(i as u32);
        }
    }
    None
}

/// Cores that have completed CPU-local init.
pub static AP_ONLINE_COUNT: AtomicU32 = AtomicU32::new(0);

static SHUTDOWN_QUIESCE_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_QUIESCE_ACK_MASK: AtomicU64 = AtomicU64::new(0);
static REBOOT_OWNER_CORE: AtomicU32 = AtomicU32::new(u32::MAX);

/// Set by BSP during MADT/CPUID enumeration before AP startup.
static mut CPU_COUNT: u32 = 1;

/// Once from BSP before starting APs.
///
/// # Safety
/// Call once from the BSP before any AP is started; mutates a global read by
/// all cores. No concurrent access is permitted.
pub unsafe fn set_cpu_count(n: u32) {
    CPU_COUNT = n;
}

pub fn cpu_count() -> u32 {
    unsafe { CPU_COUNT }
}

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
#[allow(dead_code)]
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

const IA32_FS_BASE: u32 = 0xC000_0100;
const IA32_GS_BASE: u32 = 0xC000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// Write the user TLS base (IA32_FS_BASE). FSGSBASE is off, so this MSR is the
/// sole FS-base mutation path and `Process.tls_base` stays authoritative.
///
/// # Safety
/// `tp` must be canonical (Intel SDM Vol 3 §2.2.1: writing a non-canonical
/// IA32_FS_BASE `#GP`s). Enforced at the syscall boundary via `USER_ADDR_LIMIT`.
#[inline]
pub unsafe fn set_user_tls_base(tp: u64) {
    wrmsr(IA32_FS_BASE, tp);
}

/// # Safety
/// Once on BSP, after GDT, before IF=1.
pub unsafe fn init_bsp(lapic_id: u32, lapic_base: u64) {
    // BSP = core 0.
    let idx = 0usize;

    let pcpu = &mut PER_CPU_ARRAY[idx];
    pcpu.self_ptr = pcpu as *const PerCpu as u64;
    pcpu.cpu_id = lapic_id;
    pcpu.current_pid = 0;
    pcpu.lapic_base = lapic_base;
    pcpu.core_index = idx as u32;
    pcpu.online = true;

    map_lapic_id(lapic_id, idx as u8);

    let addr = pcpu as *const PerCpu as u64;
    wrmsr(IA32_GS_BASE, addr);
    // Kernel GS-base = 0; swapped in on ring transitions.
    wrmsr(IA32_KERNEL_GS_BASE, 0);

    AP_ONLINE_COUNT.store(1, Ordering::SeqCst);

    debug_assert_offsets();
}

/// `stack_top` must be `base + AP_STACK_SIZE` (exact top BSP allocated). Stored
/// in `boot_kernel_rsp`; scheduler restores RSP to this on AP return-to-idle
/// after descheduling a user process. Wrong value here triple-faults on first idle return.
///
/// # Safety
/// Once per AP, on AP stack, IF=0.
pub unsafe fn init_ap(core_idx: u32, lapic_id: u32, lapic_base: u64, stack_top: u64) {
    let idx = core_idx as usize;
    assert!(idx < MAX_CPUS);

    let pcpu = &mut PER_CPU_ARRAY[idx];
    pcpu.self_ptr = pcpu as *const PerCpu as u64;
    pcpu.cpu_id = lapic_id;
    pcpu.current_pid = 0;
    pcpu.lapic_base = lapic_base;
    pcpu.core_index = core_idx;
    pcpu.online = true;

    pcpu.boot_kernel_rsp = stack_top;

    map_lapic_id(lapic_id, idx as u8);

    let addr = pcpu as *const PerCpu as u64;
    wrmsr(IA32_GS_BASE, addr);
    wrmsr(IA32_KERNEL_GS_BASE, 0);
}

/// Hot path — avoid in the timer ISR. ASM reads `gs:` directly.
///
/// # Safety
/// `GS_BASE` must already point at this core's `PerCpu` (set during per-CPU
/// init). Returns a `&'static mut` to per-core state; caller must not alias it
/// across cores.
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

/// # Safety
/// `idx` must be `< MAX_CPUS`. Returns a `&'static mut` into the per-CPU array;
/// caller must ensure no other reference to the same slot is live.
#[inline(always)]
pub unsafe fn by_index(idx: u32) -> &'static mut PerCpu {
    &mut PER_CPU_ARRAY[idx as usize]
}

/// # Safety
/// Returns a `&'static mut` into the per-CPU array; caller must ensure no other
/// reference to the same slot is live.
#[inline(always)]
pub unsafe fn by_lapic_id(lapic_id: u32) -> Option<&'static mut PerCpu> {
    lapic_id_to_index(lapic_id).map(|idx| &mut PER_CPU_ARRAY[idx as usize])
}

/// # Safety
/// `GS_BASE` must point at this core's `PerCpu` (set during per-CPU init).
#[inline(always)]
pub unsafe fn current_core_index() -> u32 {
    current().core_index
}

/// # Safety
/// `GS_BASE` must point at this core's `PerCpu` (set during per-CPU init).
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

/// # Safety
/// `GS_BASE` must point at this core's `PerCpu` (set during per-CPU init).
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

/// # Safety
/// `GS_BASE` must point at this core's `PerCpu` (set during per-CPU init).
#[inline(always)]
pub unsafe fn set_current_pid(pid: u32) {
    core::arch::asm!(
        "mov gs:[0x0C], {0:e}",
        in(reg) pid,
        options(nostack, preserves_flags),
    );
}

/// >1 core online.
#[inline(always)]
pub fn smp_active() -> bool {
    AP_ONLINE_COUNT.load(Ordering::Relaxed) > 1
}

#[inline(always)]
pub fn shutdown_quiesce_requested() -> bool {
    SHUTDOWN_QUIESCE_REQUESTED.load(Ordering::Acquire)
}

#[inline(always)]
pub fn shutdown_quiesce_ack(core_idx: u32) {
    if core_idx < 64 {
        SHUTDOWN_QUIESCE_ACK_MASK.fetch_or(1u64 << core_idx, Ordering::Release);
    }
}

pub fn request_shutdown_quiesce() {
    let owner = reboot_owner().unwrap_or(0);
    let owner_mask = if owner < 64 { 1u64 << owner } else { 1u64 };
    SHUTDOWN_QUIESCE_ACK_MASK.store(owner_mask, Ordering::Release);
    SHUTDOWN_QUIESCE_REQUESTED.store(true, Ordering::Release);
}

pub fn wait_for_shutdown_quiesce(timeout_ms: u64) -> bool {
    let online = AP_ONLINE_COUNT.load(Ordering::Acquire) as usize;
    if online <= 1 {
        return true;
    }

    let expected = if online >= 64 {
        u64::MAX
    } else {
        (1u64 << online) - 1
    };

    let tsc_hz = crate::cpu::tsc::tsc_frequency();
    let deadline = if tsc_hz > 0 {
        let ticks_per_ms = (tsc_hz / 1000).max(1);
        Some(crate::cpu::tsc::read_tsc().saturating_add(timeout_ms.saturating_mul(ticks_per_ms)))
    } else {
        None
    };

    // Fallback bound so teardown cannot deadlock if TSC isn't calibrated.
    let mut fallback_spins = timeout_ms.saturating_mul(200_000).max(200_000);

    loop {
        let acked = SHUTDOWN_QUIESCE_ACK_MASK.load(Ordering::Acquire);
        if (acked & expected) == expected {
            return true;
        }

        if let Some(d) = deadline {
            if crate::cpu::tsc::read_tsc() >= d {
                return false;
            }
        } else {
            if fallback_spins == 0 {
                return false;
            }
            fallback_spins -= 1;
        }

        core::hint::spin_loop();
    }
}

#[inline(always)]
pub fn set_reboot_owner(core_idx: u32) {
    REBOOT_OWNER_CORE.store(core_idx, Ordering::Release);
}

#[inline(always)]
pub fn clear_reboot_owner() {
    REBOOT_OWNER_CORE.store(u32::MAX, Ordering::Release);
}

#[inline(always)]
pub fn reboot_owner() -> Option<u32> {
    let owner = REBOOT_OWNER_CORE.load(Ordering::Acquire);
    if owner == u32::MAX {
        None
    } else {
        Some(owner)
    }
}

#[inline(always)]
pub fn is_reboot_owner(core_idx: u32) -> bool {
    REBOOT_OWNER_CORE.load(Ordering::Acquire) == core_idx
}

/// Runtime offset check at BSP init (no const `offset_of!` on stable). Fires
/// once before any AP or IRQ could use wrong offsets.
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
