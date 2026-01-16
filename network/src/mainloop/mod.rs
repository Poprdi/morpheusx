//! Main loop module.
//!
//! State machine orchestration for network operations (DHCP, DNS, HTTP, etc).
//!
//! # Architecture
//!
//! ## New Modular State Machine (preferred)
//! - `state` - State trait and StepResult for state machine
//! - `states` - Individual state implementations (reusable building blocks)
//! - `serial` - Serial output primitives
//! - `adapter` - smoltcp Device adapter
//! - `context` - Shared context between states
//! - `disk_writer` - Buffered disk writer for streaming writes
//! - `orchestrator` - High-level entry points (`download`, `download_with_config`)
//!
//! ## Legacy Entry Point (backward compatible)
//! - `bare_metal` - Monolithic function with full download + manifest support
//! - `phases` - Low-level RX/TX phases  
//! - `runner` - Old iteration runner
//!
//! # Usage
//!
//! ```ignore
//! // New API (self-contained, no handoff required):
//! use morpheus_network::mainloop::{download, DownloadResult};
//! let result = download(&mut driver, "http://example.com/file.iso", tsc_freq);
//!
//! // Legacy API (for bootloader with handoff):
//! use morpheus_network::mainloop::{bare_metal_main, BareMetalConfig, RunResult};
//! let result = unsafe { bare_metal_main(handoff, config) };
//! ```

// New modular state machine
pub mod adapter;
pub mod context;
pub mod disk_writer;
pub mod serial;
pub mod state;
pub mod states;
pub mod orchestrator;

// Legacy modules (deprecated, will be removed)
pub mod bare_metal;
pub mod phases;
pub mod runner;

// New re-exports
pub use adapter::SmoltcpAdapter;
pub use context::{Context, DownloadConfig, Timeouts, get_tsc};
pub use disk_writer::DiskWriter;
pub use serial::{print, println, print_hex, print_u32, print_mac, print_ipv4};
pub use state::{State, StepResult};
pub use states::{InitState, DhcpState, DnsState, ConnectState, HttpState, DoneState, FailedState};
pub use orchestrator::{download, download_with_config, DownloadResult};

// Legacy re-exports (deprecated)
pub use bare_metal::{
    bare_metal_main, run_full_download, serial_print, serial_print_hex, serial_println,
    BareMetalConfig, RunResult,
};
pub use phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
pub use runner::{run_iteration, IterationResult, MainLoopConfig};
