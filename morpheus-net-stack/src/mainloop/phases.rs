//! Main-loop phases: RX refill, smoltcp poll (exactly once), TX drain, app
//! step, TX completions. Target <1 ms, max 5 ms per iteration.

use morpheus_nic::traits::NetworkDriver;

/// Max packets to send per iteration in the TX-drain phase.
pub const TX_BUDGET: usize = 16;

pub fn phase1_rx_refill<D: NetworkDriver>(device: &mut D) {
    device.refill_rx_queue();
}

pub fn phase5_tx_completions<D: NetworkDriver>(device: &mut D) {
    device.collect_tx_completions();
}

/// True if the iteration exceeded the warning threshold.
#[cfg(target_arch = "x86_64")]
pub fn check_timing_warning(start_tsc: u64, warning_threshold_ticks: u64) -> bool {
    let now = morpheus_hal_x86_64::asm::tsc::read_tsc();
    let elapsed = now.wrapping_sub(start_tsc);
    elapsed > warning_threshold_ticks
}

#[cfg(not(target_arch = "x86_64"))]
pub fn check_timing_warning(_start_tsc: u64, _warning_threshold_ticks: u64) -> bool {
    false
}
