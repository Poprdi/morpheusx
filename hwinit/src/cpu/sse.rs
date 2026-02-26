//! SSE/FPU initialization.
//!
//! Configures CR0 and CR4 so that SSE (XMM0-XMM15) and x87 FPU instructions
//! execute without faulting.  Must be called once during early boot, before
//! any code that uses floating point or SIMD.
//!
//! ## Bits touched
//!
//! | Register | Bit | Name        | Value | Why                                     |
//! |----------|-----|-------------|-------|-----------------------------------------|
//! | CR0      |  1  | MP          |   1   | Monitor coprocessor (required with SSE) |
//! | CR0      |  2  | EM          |   0   | No FPU emulation                        |
//! | CR0      |  3  | TS          |   0   | No lazy-switch #NM — we save eagerly    |
//! | CR4      |  9  | OSFXSR      |   1   | OS supports FXSAVE/FXRSTOR              |
//! | CR4      | 10  | OSXMMEXCPT  |   1   | OS handles #XF (SIMD FP exceptions)     |

use crate::serial::puts;

const CR0_MP: u64 = 1 << 1;
const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;

/// Enable SSE and x87 FPU for the kernel and all user processes.
///
/// # Safety
/// Must be called once during single-threaded boot, before interrupts are
/// enabled and before any floating-point code runs.
pub unsafe fn enable_sse() {
    // ── CR0: clear EM and TS, set MP ─────────────────────────────────────
    let cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    let cr0_new = (cr0 | CR0_MP) & !(CR0_EM | CR0_TS);
    if cr0_new != cr0 {
        core::arch::asm!("mov cr0, {}", in(reg) cr0_new, options(nomem, nostack));
    }

    // ── CR4: set OSFXSR and OSXMMEXCPT ───────────────────────────────────
    let cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    let cr4_new = cr4 | CR4_OSFXSR | CR4_OSXMMEXCPT;
    if cr4_new != cr4 {
        core::arch::asm!("mov cr4, {}", in(reg) cr4_new, options(nomem, nostack));
    }

    puts("[SSE] enabled — CR0.EM=0 CR0.TS=0 CR4.OSFXSR=1 CR4.OSXMMEXCPT=1\n");
}
