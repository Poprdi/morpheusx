//! Bootloader-side net init orchestration. Coordinates `dma-pool` and
//! `morpheus_network`; errors land in a ring buffer the bootstrap UI drains.

mod config;
mod error;
mod init;
mod ring_buffer;
mod status;

pub use config::{InitConfig, ECAM_BASE_QEMU_I440FX, ECAM_BASE_QEMU_Q35};
pub use error::{NetInitError, NetInitResult};
pub use ring_buffer::{
    debug_log, drain_network_logs, error_log, error_log_available, error_log_clear,
    error_log_count, error_log_pop, ErrorLogEntry, InitStage,
};
pub use status::NetworkStatus;
