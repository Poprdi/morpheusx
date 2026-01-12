//! UEFI utilities for boot services and firmware interaction.

pub mod esp;
pub mod helpers;
pub mod timing;

pub use esp::find_esp_lba;
pub use helpers::{exit_boot_services_with_retry, leak_string};
pub use timing::calibrate_tsc_with_stall;
