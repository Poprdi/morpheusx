//! Main loop runner.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6.2

use super::phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
use crate::driver::NetworkDriver;

/// Main loop configuration.
pub struct MainLoopConfig {
    /// TSC frequency (ticks per second).
    pub tsc_freq: u64,
    /// Warning threshold for iteration timing (ticks).
    pub timing_warning_ticks: u64,
}

impl MainLoopConfig {
    /// Create from TSC frequency.
    pub fn new(tsc_freq: u64) -> Self {
        Self {
            tsc_freq,
            // 5ms warning threshold
            timing_warning_ticks: tsc_freq / 200,
        }
    }

    /// Convert TSC ticks to milliseconds.
    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks * 1000 / self.tsc_freq
    }
}

/// Single iteration result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterationResult {
    /// Continue running.
    Continue,
    /// Application completed successfully.
    Done,
    /// Application failed.
    Failed,
    /// Application timed out.
    Timeout,
}

/// Run a single main loop iteration.
///
/// This is a building block - the full main loop calls this repeatedly.
/// Useful for testing and integration.
///
/// # Arguments
/// - `device`: Network device
/// - `config`: Loop configuration
///
/// # Returns
/// Whether to continue looping.
#[cfg(target_arch = "x86_64")]
pub fn run_iteration<D: NetworkDriver>(
    device: &mut D,
    _config: &MainLoopConfig,
) -> IterationResult {
    // Phase 1: Refill RX queue
    phase1_rx_refill(device);

    // Phase 2: Would call smoltcp poll here
    // (requires smoltcp integration - handled by caller)

    // Phase 3: TX drain
    // (handled by smoltcp adapter)

    // Phase 4: App state machine step
    // (handled by caller)

    // Phase 5: Collect TX completions
    phase5_tx_completions(device);

    IterationResult::Continue
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run_iteration<D: NetworkDriver>(
    _device: &mut D,
    _config: &MainLoopConfig,
) -> IterationResult {
    IterationResult::Continue
}

/// Get current TSC value.
#[cfg(target_arch = "x86_64")]
pub fn get_tsc() -> u64 {
    unsafe { crate::asm::core::tsc::read_tsc() }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn get_tsc() -> u64 {
    0
}
