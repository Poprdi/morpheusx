//! TSC frequency cache + PIT calibration.

pub use crate::asm::tsc::*;

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// PIT channel 2 calibration; returns Hz.
    fn asm_tsc_calibrate_pit() -> u64;
}

/// Calibrates against the 8254 PIT (no UEFI deps) and caches the result in
/// `TSC_FREQUENCY_HZ` so `tsc_frequency()` consumers (uptime stamping, reset /
/// per-cpu delay loops) see it — previously the calibrated value was only
/// published to the kernel scheduler, leaving the HAL's own cache at 0.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn calibrate_tsc_pit() -> u64 {
    // SAFETY: programs the 8254 and reads the TSC; no memory effects.
    let hz = unsafe { asm_tsc_calibrate_pit() };
    set_tsc_frequency(hz);
    hz
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn calibrate_tsc_pit() -> u64 {
    0
}

/// Populated once at boot; read by apic/reset/per_cpu delay loops without a
/// back-ref into the scheduler.
static TSC_FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);

pub fn set_tsc_frequency(hz: u64) {
    TSC_FREQUENCY_HZ.store(hz, Ordering::Relaxed);
}

pub fn tsc_frequency() -> u64 {
    TSC_FREQUENCY_HZ.load(Ordering::Relaxed)
}
