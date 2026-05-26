//! Enable SSE/x87 via CR0/CR4 per AMD64 Vol 2 §11.5.
//! CR0: MP=1, clear EM+TS (we save FPU eagerly).
//! CR4: OSFXSR=1, OSXMMEXCPT=1 (FXSAVE/FXRSTOR + #XF).

use crate::serial::log_ok;
use core::sync::atomic::{AtomicBool, Ordering};

const CR0_MP: u64 = 1 << 1;
const CR0_EM: u64 = 1 << 2;
const CR0_TS: u64 = 1 << 3;
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;
static SSE_LOGGED: AtomicBool = AtomicBool::new(false);

/// # Safety
/// Call once during single-threaded boot before any FP/SIMD code runs.
pub unsafe fn enable_sse() {
    let cr0: u64;
    core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
    let cr0_new = (cr0 | CR0_MP) & !(CR0_EM | CR0_TS);
    if cr0_new != cr0 {
        core::arch::asm!("mov cr0, {}", in(reg) cr0_new, options(nomem, nostack));
    }

    let cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    let cr4_new = cr4 | CR4_OSFXSR | CR4_OSXMMEXCPT;
    if cr4_new != cr4 {
        core::arch::asm!("mov cr4, {}", in(reg) cr4_new, options(nomem, nostack));
    }

    if !SSE_LOGGED.swap(true, Ordering::AcqRel) {
        log_ok("SSE", 701, "SIMD/FPU enabled");
    }
}
