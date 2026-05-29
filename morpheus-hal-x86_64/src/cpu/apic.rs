//! Local APIC driver: init, EOI, IPI, timer, INIT/SIPI for SMP.
//! MMIO base from MSR 0x1B (default 0xFEE0_0000), identity-mapped UC.

use crate::asm::pio::outb;
use crate::cpu::per_cpu;
use crate::serial::log_warn;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// BSP-calibrated timer init count. APs reuse this to skip the PIT race
/// (multiple APs on PIT ch2 simultaneously deadlock). 0 = uncalibrated.
static LAPIC_TIMER_INIT_COUNT: AtomicU32 = AtomicU32::new(0);

const LAPIC_ID: u32 = 0x020;
const LAPIC_VER: u32 = 0x030;
const LAPIC_TPR: u32 = 0x080;
const LAPIC_EOI: u32 = 0x0B0;
const LAPIC_SVR: u32 = 0x0F0;
const LAPIC_ICR_LO: u32 = 0x300;
const LAPIC_ICR_HI: u32 = 0x310;
const LAPIC_LVT_TIMER: u32 = 0x320;
const LAPIC_TIMER_INIT: u32 = 0x380;
const LAPIC_TIMER_CURRENT: u32 = 0x390;
const LAPIC_TIMER_DIV: u32 = 0x3E0;

// SVR bits
const SVR_ENABLE: u32 = 1 << 8;
const SVR_SPURIOUS_VECTOR: u32 = 0xFF;

// LVT timer modes
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_MASKED: u32 = 1 << 16;

// ICR delivery modes
const ICR_INIT: u32 = 0x500;
const ICR_STARTUP: u32 = 0x600;
const ICR_LEVEL_ASSERT: u32 = 0x4000;
const ICR_LEVEL_DEASSERT: u32 = 0x0000;
const ICR_TRIGGER_LEVEL: u32 = 1 << 15;
const ICR_DELIVERY_STATUS: u32 = 1 << 12;

pub const DEFAULT_LAPIC_BASE: u64 = 0xFEE0_0000;

/// Timer vector. Reuses the legacy PIT vector so existing ISR fires.
pub const TIMER_VECTOR: u8 = 0x20;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const IA32_X2APIC_ICR_MSR: u32 = 0x830;
const IA32_X2APIC_ID_MSR: u32 = 0x802;

/// Probed from MSR 0x1B during BSP init; read-only after.
static mut LAPIC_BASE_ACTUAL: u64 = DEFAULT_LAPIC_BASE;
static X2APIC_ENABLED: AtomicBool = AtomicBool::new(false);

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
    ((hi as u64) << 32) | lo as u64
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
unsafe fn wrmsr_parts(msr: u32, lo: u32, hi: u32) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, nomem),
    );
}

#[inline(always)]
fn x2apic_enabled() -> bool {
    X2APIC_ENABLED.load(Ordering::Relaxed)
}

#[inline(always)]
pub fn is_x2apic_mode() -> bool {
    x2apic_enabled()
}

/// Read IA32_APIC_BASE, record base + x2APIC mode. Call once on BSP.
///
/// # Safety
/// Single-threaded BSP init.
pub unsafe fn probe_lapic_base() -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") IA32_APIC_BASE_MSR,
        out("eax") lo,
        out("edx") hi,
        options(nostack, nomem),
    );
    let raw = (hi as u64) << 32 | lo as u64;
    let x2 = (raw & (1 << 10)) != 0;
    X2APIC_ENABLED.store(x2, Ordering::Relaxed);
    let base = raw & 0xFFFF_FFFF_FFFF_F000;
    LAPIC_BASE_ACTUAL = base;

    base
}

/// Returns DEFAULT_LAPIC_BASE until `probe_lapic_base` runs.
#[inline(always)]
pub fn lapic_base() -> u64 {
    unsafe { LAPIC_BASE_ACTUAL }
}

#[inline(always)]
unsafe fn lapic_read(base: u64, reg: u32) -> u32 {
    let ptr = (base + reg as u64) as *const u32;
    core::ptr::read_volatile(ptr)
}

#[inline(always)]
unsafe fn lapic_write(base: u64, reg: u32, val: u32) {
    let ptr = (base + reg as u64) as *mut u32;
    core::ptr::write_volatile(ptr, val);
}

/// Read this CPU's LAPIC ID from hardware.
///
/// # Safety
/// LAPIC must be initialized and its MMIO base identity-mapped (or x2APIC MSRs
/// available). Touches APIC hardware on the current core only.
pub unsafe fn read_lapic_id() -> u32 {
    if x2apic_enabled() {
        rdmsr(IA32_X2APIC_ID_MSR) as u32
    } else {
        lapic_read(lapic_base(), LAPIC_ID) >> 24
    }
}

/// CPUID.01h:EDX.APIC[bit 9].
pub fn apic_available() -> bool {
    let edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("edx") edx,
            out("eax") _,
            out("ecx") _,
            options(nostack),
        );
    }
    (edx & (1 << 9)) != 0
}

/// SVR.enable + TPR=0; identity-map LAPIC page UC. Call once on BSP after paging.
///
/// # Safety
/// BSP, post-GDT/IDT/paging.
pub unsafe fn init_bsp() {
    let base = lapic_base();

    if crate::paging::is_paging_initialized() {
        let _ = crate::paging::kmap_mmio(base, 0x1000);
    }

    let svr = lapic_read(base, LAPIC_SVR);
    lapic_write(base, LAPIC_SVR, svr | SVR_ENABLE | SVR_SPURIOUS_VECTOR);
    lapic_write(base, LAPIC_TPR, 0);

    let _id = lapic_read(base, LAPIC_ID) >> 24;
    let _ver = lapic_read(base, LAPIC_VER);
}

/// Per-AP LAPIC enable, called from `ap_rust_entry`.
///
/// # Safety
/// Once per AP on its own stack.
pub unsafe fn init_ap() {
    let base = lapic_base();

    let svr = lapic_read(base, LAPIC_SVR);
    lapic_write(base, LAPIC_SVR, svr | SVR_ENABLE | SVR_SPURIOUS_VECTOR);
    lapic_write(base, LAPIC_TPR, 0);

    let _id = lapic_read(base, LAPIC_ID) >> 24;
}

/// Write 0 to EOI. Call at end of every LAPIC-sourced ISR.
///
/// # Safety
/// LAPIC must be initialized with its MMIO base identity-mapped. Call exactly
/// once at the end of a LAPIC-sourced interrupt service routine.
#[inline(always)]
pub unsafe fn send_eoi() {
    lapic_write(lapic_base(), LAPIC_EOI, 0);
}

/// Calibrate against PIT ch2 (1.193182 MHz), arm periodic mode.
/// APs reuse BSP init_count to avoid simultaneous PIT ch2 contention.
///
/// # Safety
/// Per-core, post LAPIC enable. PIT must be accessible.
pub unsafe fn setup_timer(target_hz: u32) {
    let base = lapic_base();

    let cached = LAPIC_TIMER_INIT_COUNT.load(Ordering::Acquire);
    if cached != 0 {
        lapic_write(base, LAPIC_LVT_TIMER, TIMER_PERIODIC | TIMER_VECTOR as u32);
        lapic_write(base, LAPIC_TIMER_DIV, 0x03);
        lapic_write(base, LAPIC_TIMER_INIT, cached);
        return;
    }

    lapic_write(base, LAPIC_TIMER_DIV, 0x03); // divide by 16

    // PIT oscillator = 1,193,182 Hz; 10ms window.
    const PIT_HZ: u32 = 1_193_182;
    const CALIBRATION_MS: u32 = 10;
    const PIT_TICKS: u32 = (PIT_HZ / 1000) * CALIBRATION_MS;

    outb(0x61, (crate::asm::pio::inb(0x61) & 0xFD) | 0x01); // gate high, speaker off
    outb(0x43, 0xB0); // ch2, lobyte/hibyte, mode 0 (one-shot)
    outb(0x42, (PIT_TICKS & 0xFF) as u8);
    outb(0x42, ((PIT_TICKS >> 8) & 0xFF) as u8);

    // Rearm PIT gate BEFORE LAPIC start: each port out is ~1us on real hw;
    // reversing order inflates the measured LAPIC frequency 5-15% and
    // shrinks AP SIPI delays below spec.
    let v = crate::asm::pio::inb(0x61) & 0xFE;
    outb(0x61, v);
    outb(0x61, v | 1);

    lapic_write(base, LAPIC_TIMER_INIT, 0xFFFF_FFFF);

    // Bounded PIT spin: previously this could hang forever if PIT ch2 was
    // absent (some modern boards), masked, or wedged. Cap by iteration count
    // since TSC is not yet calibrated. ~1us per inb on real hw → 1M
    // iterations ≈ 1s upper bound, far past the 10ms window we expect.
    crate::serial::checkpoint("lapic-pit-spin");
    let mut pit_timed_out = false;
    let mut spins: u32 = 0;
    while crate::asm::pio::inb(0x61) & 0x20 == 0 {
        spins = spins.wrapping_add(1);
        if spins >= 1_000_000 {
            pit_timed_out = true;
            break;
        }
        core::hint::spin_loop();
    }
    crate::serial::checkpoint("lapic-pit-done");

    let elapsed = 0xFFFF_FFFFu32 - lapic_read(base, LAPIC_TIMER_CURRENT);

    lapic_write(base, LAPIC_LVT_TIMER, TIMER_MASKED);

    if pit_timed_out || elapsed == 0 {
        // PIT unusable. Fall back to an assumed-bus heuristic so the
        // scheduler still ticks (wrong rate beats a hang). Numbers chosen
        // for a ~100 MHz LAPIC bus / divisor 16 → 6.25 MHz effective.
        log_warn(
            "LAPIC",
            723,
            if pit_timed_out {
                "PIT spin timed out; using fallback timer rate"
            } else {
                "timer calibration returned zero ticks; using fallback rate"
            },
        );
        const FALLBACK_TICKS_PER_SEC: u64 = 6_250_000;
        let init_count = (FALLBACK_TICKS_PER_SEC / target_hz as u64) as u32;
        LAPIC_TIMER_INIT_COUNT.store(init_count, Ordering::Release);
        lapic_write(base, LAPIC_LVT_TIMER, TIMER_PERIODIC | TIMER_VECTOR as u32);
        lapic_write(base, LAPIC_TIMER_DIV, 0x03);
        lapic_write(base, LAPIC_TIMER_INIT, init_count);
        return;
    }

    // Hardware applies divisor already; ticks/s = elapsed * (1000 / cal_ms).
    let ticks_per_second = elapsed as u64 * (1000 / CALIBRATION_MS as u64);
    let init_count = (ticks_per_second / target_hz as u64) as u32;

    let _ = (elapsed, init_count, target_hz);

    // Publish before arming so APs never touch the PIT.
    LAPIC_TIMER_INIT_COUNT.store(init_count, Ordering::Release);

    lapic_write(base, LAPIC_LVT_TIMER, TIMER_PERIODIC | TIMER_VECTOR as u32);
    lapic_write(base, LAPIC_TIMER_DIV, 0x03);
    lapic_write(base, LAPIC_TIMER_INIT, init_count);
}

/// Mask all 8259 IRQs. Required once LAPIC is the interrupt source.
///
/// # Safety
/// Performs raw `out` to the 8259 PIC ports. Call once during init after the
/// LAPIC is up and no code still depends on legacy PIC delivery.
pub unsafe fn disable_pic8259() {
    outb(0x21, 0xFF);
    outb(0xA1, 0xFF);
}

/// INIT IPI assert. Caller waits >=200us then `send_init_deassert`.
///
/// # Safety
/// LAPIC must be initialized with its MMIO base identity-mapped. `target_apic_id`
/// must be a valid physical APIC ID. Part of the SMP startup sequence; caller is
/// responsible for the mandated inter-IPI delays.
pub unsafe fn send_init_assert(target_apic_id: u32) {
    let base = lapic_base();

    if x2apic_enabled() {
        let icr = (target_apic_id as u64) << 32
            | (ICR_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_ASSERT) as u64;
        wrmsr(IA32_X2APIC_ICR_MSR, icr);
        wait_icr_idle(base);
        return;
    }

    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(
        base,
        LAPIC_ICR_LO,
        ICR_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_ASSERT,
    );
    wait_icr_idle(base);
}

/// INIT IPI deassert. Trigger MUST stay level or KVM treats it as assert.
///
/// # Safety
/// LAPIC must be initialized with its MMIO base identity-mapped. `target_apic_id`
/// must be a valid physical APIC ID. Must follow a prior `send_init_assert`.
pub unsafe fn send_init_deassert(target_apic_id: u32) {
    let base = lapic_base();

    if x2apic_enabled() {
        let icr = (target_apic_id as u64) << 32
            | (ICR_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_DEASSERT) as u64;
        wrmsr(IA32_X2APIC_ICR_MSR, icr);
        wait_icr_idle(base);
        return;
    }

    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(
        base,
        LAPIC_ICR_LO,
        ICR_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_DEASSERT,
    );
    wait_icr_idle(base);
}

/// SIPI. `start_page` = trampoline phys / 0x1000 (must be <1 MiB, page-aligned).
///
/// # Safety
/// LAPIC must be initialized with its MMIO base identity-mapped. `target_apic_id`
/// must be valid and `start_page` must point at a real, page-aligned trampoline
/// below 1 MiB. Must follow the INIT assert/deassert sequence.
pub unsafe fn send_sipi(target_apic_id: u32, start_page: u8) {
    let base = lapic_base();
    if x2apic_enabled() {
        let icr_lo = ICR_STARTUP | start_page as u32;
        let icr_hi = target_apic_id;
        wrmsr_parts(IA32_X2APIC_ICR_MSR, icr_lo, icr_hi);
        wait_icr_idle(base);
        return;
    }
    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(base, LAPIC_ICR_LO, ICR_STARTUP | start_page as u32);
    wait_icr_idle(base);
}

/// Poll ICR delivery-status. x2APIC ICR writes are synchronous (no poll),
/// and reading MSR 0x830 #GPs on some CPUs.
#[inline]
unsafe fn wait_icr_idle(base: u64) {
    if x2apic_enabled() {
        return;
    }

    let mut timeout = 10_000u32;
    while (lapic_read(base, LAPIC_ICR_LO) & ICR_DELIVERY_STATUS) != 0 {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            log_warn("LAPIC", 726, "ICR delivery timeout");
            break;
        }
    }
}

/// TSC-based busy wait. Spin fallback if TSC freq is bogus. Watchdog
/// caps spin count: bad calibration must not turn a 10 ms wait into hours.
///
/// # Safety
/// Reads the TSC via `rdtsc`; requires TSC support (always present on supported
/// CPUs). Pure busy-spin with no side effects beyond burning cycles.
pub unsafe fn delay_us(us: u64) {
    let freq = crate::cpu::tsc::tsc_frequency();
    if (1_000_000..=10_000_000_000).contains(&freq) {
        let cycles_per_us = freq / 1_000_000;
        if cycles_per_us == 0 {
            for _ in 0..(us * 1000) {
                core::hint::spin_loop();
            }
            return;
        }

        let start = crate::cpu::tsc::read_tsc();
        let delta = cycles_per_us.saturating_mul(us);
        let target = start.saturating_add(delta);

        // ~10 cycles/iter (RDTSC+cmp+PAUSE) is the lower bound; using it
        // ensures the watchdog never trips before the TSC target.
        let iters_per_us = (cycles_per_us / 10).max(1);
        let max_spins_u64 = us.saturating_mul(iters_per_us).clamp(10_000, 1_000_000_000);
        let max_spins = max_spins_u64 as u32;
        let mut spins = 0u32;
        while crate::cpu::tsc::read_tsc() < target {
            core::hint::spin_loop();
            spins = spins.wrapping_add(1);
            if spins >= max_spins {
                log_warn("LAPIC", 727, "delay_us watchdog tripped; falling back");
                break;
            }
        }
    } else {
        // ~1us/iter at ~1 GHz; best effort.
        for _ in 0..(us * 1000) {
            core::hint::spin_loop();
        }
    }
}

/// Logical CPU count via CPUID. Leaf 0xB (x2APIC topology) preferred,
/// falls back to leaf 1. Returns >=1.
pub fn detect_cpu_count() -> u32 {
    let max_leaf: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "xor eax, eax",
            "cpuid",
            "pop rbx",
            out("eax") max_leaf,
            out("ecx") _,
            out("edx") _,
            options(nostack),
        );
    }

    if max_leaf >= 0xB {
        let ebx: u32;
        unsafe {
            core::arch::asm!(
                "push rbx",
                "mov eax, 0xB",
                "xor ecx, ecx",    // SMT level
                "cpuid",
                "mov {0:e}, ebx",
                "pop rbx",
                out(reg) ebx,
                out("eax") _,
                out("ecx") _,
                out("edx") _,
                options(nostack),
            );
        }
        if ebx > 0 {
            let ebx1: u32;
            unsafe {
                core::arch::asm!(
                    "push rbx",
                    "mov eax, 0xB",
                    "mov ecx, 1",   // core level
                    "cpuid",
                    "mov {0:e}, ebx",
                    "pop rbx",
                    out(reg) ebx1,
                    out("eax") _,
                    out("ecx") _,
                    out("edx") _,
                    options(nostack),
                );
            }
            if ebx1 > 0 {
                return ebx1.min(per_cpu::MAX_CPUS as u32);
            }
            return ebx.min(per_cpu::MAX_CPUS as u32);
        }
    }

    // Leaf 1 fallback: EBX[23:16] = max logical processors.
    if max_leaf >= 1 {
        let ebx: u32;
        unsafe {
            core::arch::asm!(
                "push rbx",
                "mov eax, 1",
                "cpuid",
                "mov {0:e}, ebx",
                "pop rbx",
                out(reg) ebx,
                out("eax") _,
                out("ecx") _,
                out("edx") _,
                options(nostack),
            );
        }
        let count = (ebx >> 16) & 0xFF;
        if count > 0 {
            return count.min(per_cpu::MAX_CPUS as u32);
        }
    }

    1
}
