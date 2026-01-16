//! Main loop module.
//!
//! State machine orchestration for network operations (DHCP, DNS, HTTP, etc).
//!
//! # Architecture
//! - `state` - State trait and StepResult for state machine
//! - `states` - Individual state implementations (reusable building blocks)
//! - `serial` - Serial output primitives
//! - `adapter` - smoltcp Device adapter
//! - `context` - Shared context between states
//! - `orchestrator` - High-level entry points
//!
//! # Legacy (deprecated)
//! - `bare_metal` - Monolithic 2800+ line function (being replaced)
//! - `phases` - Low-level RX/TX phases
//! - `runner` - Old iteration runner

// New modular state machine
pub mod adapter;
pub mod context;
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
pub use serial::{print, println, print_hex, print_u32, print_mac, print_ipv4};
pub use state::{State, StepResult};
pub use states::{InitState, DhcpState, DnsState, ConnectState, HttpState, DoneState, FailedState};
pub use orchestrator::{download, DownloadResult};

// Legacy re-exports (deprecated)
pub use bare_metal::{
    bare_metal_main, run_full_download, serial_print, serial_print_hex, serial_println,
    BareMetalConfig, RunResult,
};
pub use phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
pub use runner::{run_iteration, IterationResult, MainLoopConfig};
