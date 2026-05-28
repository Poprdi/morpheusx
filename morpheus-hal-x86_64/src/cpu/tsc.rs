//! TSC frequency cache + PIT calibration.

pub use crate::asm::tsc::*;

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// PIT channel 2 calibration; returns Hz.
    fn asm_tsc_calibrate_pit() -> u64;
}

/// Talks to 8254 PIT directly; no UEFI deps.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn calibrate_tsc_pit() -> u64 {
    unsafe { asm_tsc_calibrate_pit() }
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
