//! Main loop phase implementations.
//!
//! # 5-Phase Structure
//! 1. RX Refill (~20µs)
//! 2. smoltcp Poll - EXACTLY ONCE (~200µs)
//! 3. TX Drain (~40µs)
//! 4. App State Step (~400µs)
//! 5. TX Completions (~20µs)
//!
//! Target: <1ms per iteration
//! Maximum: 5ms per iteration
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6.2, §6.3

use crate::driver::NetworkDriver;

/// TX budget per iteration (max packets to send in Phase 3).
pub const TX_BUDGET: usize = 16;

/// Phase 1: Refill RX queue.
///
/// Ensures device has buffers to receive into.
/// Budget: ~20µs
pub fn phase1_rx_refill<D: NetworkDriver>(device: &mut D) {
    device.refill_rx_queue();
}

/// Phase 5: Collect TX completions.
///
/// Reclaims TX buffers for reuse.
/// Budget: ~20µs
pub fn phase5_tx_completions<D: NetworkDriver>(device: &mut D) {
    device.collect_tx_completions();
}

/// Check if timing warning should be emitted.
///
/// Returns true if iteration exceeded warning threshold.
#[cfg(target_arch = "x86_64")]
pub fn check_timing_warning(start_tsc: u64, warning_threshold_ticks: u64) -> bool {
    let now = unsafe { crate::asm::core::tsc::read_tsc() };
    let elapsed = now.wrapping_sub(start_tsc);
    elapsed > warning_threshold_ticks
}

#[cfg(not(target_arch = "x86_64"))]
pub fn check_timing_warning(_start_tsc: u64, _warning_threshold_ticks: u64) -> bool {
    false
}
