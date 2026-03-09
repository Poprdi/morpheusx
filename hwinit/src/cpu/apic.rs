//! Local APIC driver.
//!
//! Provides LAPIC initialization, EOI, IPI, and timer setup.
//! The LAPIC is memory-mapped at 0xFEE0_0000 (default base).
//! Identity-mapped in our page tables, marked uncacheable.
//!
//! For SMP we also handle INIT/SIPI sequences to bring up AP cores.

use crate::cpu::per_cpu;
use crate::cpu::pio::outb;
use crate::serial::{put_hex32, puts};

// ── LAPIC register offsets ───────────────────────────────────────────────

const LAPIC_ID: u32 = 0x020;
const LAPIC_VER: u32 = 0x030;
const LAPIC_TPR: u32 = 0x080; // task priority
const LAPIC_EOI: u32 = 0x0B0;
const LAPIC_SVR: u32 = 0x0F0; // spurious vector
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

/// Default LAPIC physical base address.
pub const DEFAULT_LAPIC_BASE: u64 = 0xFEE0_0000;

/// Timer interrupt vector — same as PIT was on (0x20) so the same ISR runs.
pub const TIMER_VECTOR: u8 = 0x20;

/// IA32_APIC_BASE MSR — contains the actual LAPIC physical base address.
const IA32_APIC_BASE_MSR: u32 = 0x1B;

/// Actual LAPIC base, probed from MSR 0x1B. written once during BSP init.
static mut LAPIC_BASE_ACTUAL: u64 = DEFAULT_LAPIC_BASE;

/// Probe the LAPIC base address from IA32_APIC_BASE MSR.
/// Stores the result for all subsequent LAPIC access.
/// Call once, early BSP init, before any other LAPIC operations.
///
/// # Safety
/// Single-threaded BSP init. LAPIC must be identity-mapped (UEFI guarantees this).
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
    // bits [12:N] = base physical address, bits [0:11] = flags
    let base = raw & 0xFFFF_FFFF_FFFF_F000;
    LAPIC_BASE_ACTUAL = base;

    if base != DEFAULT_LAPIC_BASE {
        puts("[LAPIC] MSR 0x1B base: ");
        crate::serial::put_hex64(base);
        puts(" (relocated from default)\n");
    }

    base
}

/// Get the probed LAPIC base address. returns DEFAULT until probe_lapic_base runs.
#[inline(always)]
pub fn lapic_base() -> u64 {
    // written once during BSP init, read-only after. no lock needed.
    unsafe { LAPIC_BASE_ACTUAL }
}

// ── MMIO helpers ─────────────────────────────────────────────────────────

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

// ── Public API ───────────────────────────────────────────────────────────

/// Read the current CPU's LAPIC ID from hardware.
pub unsafe fn read_lapic_id() -> u32 {
    lapic_read(lapic_base(), LAPIC_ID) >> 24
}

/// Check if APIC is available via CPUID.
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

/// Initialize the BSP's LAPIC.
///
/// Enables the LAPIC via SVR, sets task priority to 0 (accept all),
/// and identity-maps the LAPIC MMIO page as uncacheable.
///
/// # Safety
/// Call once on BSP, after GDT/IDT/paging are set up.
pub unsafe fn init_bsp() {
    let base = lapic_base();

    // identity-map the LAPIC MMIO page as uncacheable
    if crate::paging::is_paging_initialized() {
        let _ = crate::paging::kmap_mmio(base, 0x1000);
    }

    // enable LAPIC: set SVR.enable + spurious vector
    let svr = lapic_read(base, LAPIC_SVR);
    lapic_write(base, LAPIC_SVR, svr | SVR_ENABLE | SVR_SPURIOUS_VECTOR);

    // accept all interrupts
    lapic_write(base, LAPIC_TPR, 0);

    let id = lapic_read(base, LAPIC_ID) >> 24;
    let ver = lapic_read(base, LAPIC_VER);

    puts("[LAPIC] BSP id=");
    put_hex32(id);
    puts(" ver=");
    put_hex32(ver & 0xFF);
    puts(" maxlvt=");
    put_hex32((ver >> 16) & 0xFF);
    puts("\n");
}

/// Initialize an AP's LAPIC.
///
/// Called from the AP's Rust entry point.
///
/// # Safety
/// Called once per AP on that AP's own stack.
pub unsafe fn init_ap() {
    let base = lapic_base();

    let svr = lapic_read(base, LAPIC_SVR);
    lapic_write(base, LAPIC_SVR, svr | SVR_ENABLE | SVR_SPURIOUS_VECTOR);
    lapic_write(base, LAPIC_TPR, 0);

    let id = lapic_read(base, LAPIC_ID) >> 24;
    puts("[LAPIC] AP id=");
    put_hex32(id);
    puts(" initialized\n");
}

/// Send end-of-interrupt to the local APIC.
///
/// Must be called at the end of every LAPIC-sourced interrupt handler.
/// Writing any value to the EOI register signals completion.
#[inline(always)]
pub unsafe fn send_eoi() {
    // one load from a write-once global. no branch, no per-cpu lookup.
    lapic_write(lapic_base(), LAPIC_EOI, 0);
}

/// Calibrate the LAPIC timer against the PIT and start periodic mode.
///
/// Uses PIT channel 2 as a reference clock (1.193182 MHz) to measure
/// how many LAPIC timer ticks elapse in a known period, then sets the
/// periodic mode divisor for the desired frequency.
///
/// # Safety
/// Call on each core after LAPIC is enabled.  PIT must be accessible.
pub unsafe fn setup_timer(target_hz: u32) {
    let base = lapic_base();

    // divide configuration: divide by 16
    lapic_write(base, LAPIC_TIMER_DIV, 0x03); // 0b0011 = divide by 16

    // ── Calibrate: use PIT channel 2 for a ~10ms window ──────────────────
    // PIT oscillator = 1,193,182 Hz.  10ms = 11,932 PIT ticks.
    const PIT_HZ: u32 = 1_193_182;
    const CALIBRATION_MS: u32 = 10;
    const PIT_TICKS: u32 = (PIT_HZ / 1000) * CALIBRATION_MS;

    // configure PIT channel 2 for one-shot countdown
    outb(0x61, (crate::cpu::pio::inb(0x61) & 0xFD) | 0x01); // gate high, speaker off
    outb(0x43, 0xB0); // channel 2, lobyte/hibyte, mode 0 (one-shot)
    outb(0x42, (PIT_TICKS & 0xFF) as u8);
    outb(0x42, ((PIT_TICKS >> 8) & 0xFF) as u8);

    // start LAPIC timer with max initial count
    lapic_write(base, LAPIC_TIMER_INIT, 0xFFFF_FFFF);

    // wait for PIT channel 2 to expire (bit 5 of port 0x61 goes high)
    // re-arm the gate
    let v = crate::cpu::pio::inb(0x61) & 0xFE;
    outb(0x61, v);
    outb(0x61, v | 1);

    // spin until PIT output goes high. if this hangs: PIT gate not armed, or
    // hardware has no PIT channel 2. the checkpoint tells us we entered the spin.
    crate::serial::checkpoint("lapic-pit-spin");
    while crate::cpu::pio::inb(0x61) & 0x20 == 0 {
        core::hint::spin_loop();
    }
    crate::serial::checkpoint("lapic-pit-done");

    // read how many LAPIC ticks elapsed
    let elapsed = 0xFFFF_FFFFu32 - lapic_read(base, LAPIC_TIMER_CURRENT);

    // stop the timer
    lapic_write(base, LAPIC_LVT_TIMER, TIMER_MASKED);

    if elapsed == 0 {
        puts("[LAPIC] WARNING: timer calibration returned 0 ticks\n");
        return;
    }

    // ticks per second = elapsed * (1000 / CALIBRATION_MS) * divide_factor
    // but divide_factor is already applied by hardware.
    // ticks_per_second = elapsed * (1000 / 10) = elapsed * 100
    let ticks_per_second = elapsed as u64 * (1000 / CALIBRATION_MS as u64);
    let init_count = (ticks_per_second / target_hz as u64) as u32;

    puts("[LAPIC] timer: ");
    put_hex32(elapsed);
    puts(" ticks/");
    put_hex32(CALIBRATION_MS);
    puts("ms → init=");
    put_hex32(init_count);
    puts(" for ");
    put_hex32(target_hz);
    puts("Hz\n");

    // configure periodic mode at the target frequency
    lapic_write(base, LAPIC_LVT_TIMER, TIMER_PERIODIC | TIMER_VECTOR as u32);
    lapic_write(base, LAPIC_TIMER_DIV, 0x03); // divide by 16
    lapic_write(base, LAPIC_TIMER_INIT, init_count);
}

/// Disable the 8259 PIC completely.
///
/// After switching to LAPIC we never want the PIC to fire.
/// Mask all IRQs on both chips.
pub unsafe fn disable_pic8259() {
    outb(0x21, 0xFF); // master: mask all
    outb(0xA1, 0xFF); // slave: mask all
    puts("[LAPIC] PIC8259 disabled — all interrupts via LAPIC now\n");
}

// ── IPI ──────────────────────────────────────────────────────────────────

/// Send an INIT IPI to target APIC ID.
pub unsafe fn send_init_ipi(target_apic_id: u32) {
    let base = lapic_base();

    // INIT assert
    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(base, LAPIC_ICR_LO, ICR_INIT | ICR_LEVEL_ASSERT);
    wait_icr_idle(base);

    // INIT deassert — trigger mode MUST be level (bit 15) with level=0.
    // edge-triggered deassert is treated as another INIT assertion by KVM.
    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(base, LAPIC_ICR_LO, ICR_INIT | ICR_TRIGGER_LEVEL | ICR_LEVEL_DEASSERT);
    wait_icr_idle(base);
}

/// Send a Startup IPI (SIPI) to target APIC ID.
///
/// `start_page` is the physical address / 0x1000 of the AP trampoline code.
/// Must be below 1 MiB and page-aligned (e.g., 0x8000 → start_page = 8).
pub unsafe fn send_sipi(target_apic_id: u32, start_page: u8) {
    let base = lapic_base();
    lapic_write(base, LAPIC_ICR_HI, target_apic_id << 24);
    lapic_write(
        base,
        LAPIC_ICR_LO,
        ICR_STARTUP | start_page as u32,
    );
    wait_icr_idle(base);
}

/// Wait for the ICR delivery status bit to clear.
#[inline]
unsafe fn wait_icr_idle(base: u64) {
    let mut timeout = 100_000u32;
    while lapic_read(base, LAPIC_ICR_LO) & ICR_DELIVERY_STATUS != 0 {
        core::hint::spin_loop();
        timeout -= 1;
        if timeout == 0 {
            puts("[LAPIC] WARNING: ICR delivery timeout\n");
            break;
        }
    }
}

// ── Delay helper ─────────────────────────────────────────────────────────

/// Busy-wait for approximately `us` microseconds using the TSC.
/// Falls back to a dumb loop if TSC freq is unknown.
pub unsafe fn delay_us(us: u64) {
    let freq = crate::process::scheduler::tsc_frequency();
    if freq > 0 {
        let start = crate::cpu::tsc::read_tsc();
        let target = start + (freq / 1_000_000) * us;
        while crate::cpu::tsc::read_tsc() < target {
            core::hint::spin_loop();
        }
    } else {
        // dumb fallback: ~1µs per iteration at ~1GHz
        for _ in 0..(us * 1000) {
            core::hint::spin_loop();
        }
    }
}

// ── CPU topology via CPUID ───────────────────────────────────────────────

/// Detect the number of logical processors via CPUID.
///
/// Tries leaf 0xB (x2APIC topology) first, falls back to leaf 1.
/// Returns at least 1.
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

    // try leaf 0xB (extended topology enumeration)
    if max_leaf >= 0xB {
        let ebx: u32;
        unsafe {
            core::arch::asm!(
                "push rbx",
                "mov eax, 0xB",
                "xor ecx, ecx",    // subleaf 0 (SMT level)
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
        // EBX = number of logical processors at this level
        if ebx > 0 {
            // try subleaf 1 (core level) for total count
            let ebx1: u32;
            unsafe {
                core::arch::asm!(
                    "push rbx",
                    "mov eax, 0xB",
                    "mov ecx, 1",   // subleaf 1 (core level)
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

    // fallback: leaf 1, EBX[23:16] = max logical processors
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
