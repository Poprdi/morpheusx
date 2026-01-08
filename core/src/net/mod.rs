//! Network Initialization Orchestrator
//!
//! Complete network stack initialization for the bootloader. This module
//! coordinates `dma-pool` and `morpheus_network` to bring up networking
//! and return success/failure to the bootstrap phase.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │              Bootstrap (bootloader)                         │
//! │  Calls NetworkInit::initialize(), displays result           │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              core::net (this module)                        │
//! │  Orchestrates init, manages error ring buffer               │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!         ┌────────────────────┼────────────────────┐
//!         ▼                    ▼                    ▼
//!    dma-pool           morpheus_network       ping (later)
//! ```
//!
//! # Error Handling
//!
//! All errors are logged to a ring buffer that the bootstrap UI can
//! dump if initialization fails. This includes:
//! - Core orchestration errors
//! - Network crate debug logs (forwarded from its ring buffer)
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_core::net::{NetworkInit, InitConfig, NetworkStatus};
//!
//! // Bootstrap phase
//! let config = InitConfig::default();
//! match NetworkInit::initialize(config, get_time_ms) {
//!     Ok(status) => {
//!         // Network ready! status.ip_address has our IP
//!         // Later: call ping to verify connectivity
//!     }
//!     Err(e) => {
//!         // Dump error ring buffer to UI
//!         while let Some(entry) = net::error_log_pop() {
//!             display_error(&entry);
//!         }
//!     }
//! }
//! ```

mod error;
mod config;
mod init;
mod status;
mod ring_buffer;

pub use error::{NetInitError, NetInitResult};
pub use config::InitConfig;
pub use init::NetworkInit;
pub use status::NetworkStatus;
pub use ring_buffer::{
    ErrorLogEntry, error_log, error_log_pop, error_log_available, 
    error_log_clear, error_log_count, drain_network_logs,
};

