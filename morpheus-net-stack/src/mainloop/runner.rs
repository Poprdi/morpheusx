//! Main loop runner.

use super::phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
use morpheus_nic::traits::NetworkDriver;

pub struct MainLoopConfig {
    pub tsc_freq: u64,
    pub timing_warning_ticks: u64,
}

impl MainLoopConfig {
    pub fn new(tsc_freq: u64) -> Self {
        Self {
            tsc_freq,
            // 5 ms.
            timing_warning_ticks: tsc_freq / 200,
        }
    }

    pub fn ticks_to_ms(&self, ticks: u64) -> u64 {
        ticks * 1000 / self.tsc_freq
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterationResult {
    Continue,
    Done,
    Failed,
    Timeout,
}

/// One main-loop iteration; phases 2-4 (smoltcp poll, TX drain, app step) are
/// driven by the caller.
#[cfg(target_arch = "x86_64")]
pub fn run_iteration<D: NetworkDriver>(
    device: &mut D,
    _config: &MainLoopConfig,
) -> IterationResult {
    phase1_rx_refill(device);
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

#[cfg(target_arch = "x86_64")]
pub fn get_tsc() -> u64 {
    morpheus_hal_x86_64::asm::tsc::read_tsc()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn get_tsc() -> u64 {
    0
}
