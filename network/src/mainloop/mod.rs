//! Main loop module.
//!
//! The 5-phase poll loop that drives all network activity.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6

pub mod bare_metal;
pub mod phases;
pub mod runner;

// Re-exports
pub use bare_metal::{
    bare_metal_main, run_full_download, serial_print, serial_print_hex, serial_println,
    BareMetalConfig, RunResult,
};
pub use phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
pub use runner::{get_tsc, run_iteration, IterationResult, MainLoopConfig};
