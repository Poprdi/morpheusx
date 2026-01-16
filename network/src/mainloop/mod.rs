//! Main loop module.
//!
//! State machine orchestration for network operations (DHCP, DNS, HTTP, etc).
//!
//! # Entry Point Contract
//!
//! `download_with_config()` is the sole entry point. Called AFTER:
//! - ExitBootServices
//! - hwinit has normalized hardware (bus mastering, DMA policy, cache coherency)
//! - NIC is in known-good off state
//!
//! The driver init performs a **brutal hard reset** of the NIC:
//! - Full device reset
//! - All registers cleared to defaults
//! - Loopback explicitly disabled
//! - Interrupts disabled
//! - RX/TX queues torn down and rebuilt
//!
//! This ensures pristine state regardless of what firmware/UEFI left behind.
//!
//! # State Flow
//! ```text
//! Init → GptPrep → LinkWait → DHCP → DNS → Connect → HTTP → Manifest → Done (reboot)
//! ```
//!
//! # Modules
//! - `state` - State trait and StepResult for state machine
//! - `states` - Individual state implementations
//! - `serial` - Serial output primitives (post-EBS)
//! - `adapter` - smoltcp Device adapter
//! - `context` - Shared context between states
//! - `disk_writer` - Buffered disk writer for streaming writes
//! - `orchestrator` - Entry point (`download_with_config`)
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::mainloop::{download_with_config, DownloadConfig};
//!
//! // After hwinit + driver brutally reset to pristine state:
//! let config = DownloadConfig::full(url, start_sector, 0, esp_lba, uuid, iso_name);
//! let result = download_with_config(&mut driver, config, Some(blk_device), tsc_freq);
//! ```

// State machine modules
pub mod adapter;
pub mod context;
pub mod disk_writer;
pub mod serial;
pub mod state;
pub mod states;
pub mod orchestrator;

// Support modules
pub mod phases;
pub mod runner;

// Re-exports
pub use adapter::SmoltcpAdapter;
pub use context::{Context, DownloadConfig, Timeouts};
pub use disk_writer::DiskWriter;
pub use serial::{print, println, print_hex, print_u32, print_mac, print_ipv4};
pub use state::{State, StepResult};
pub use states::{InitState, DhcpState, DnsState, ConnectState, HttpState, DoneState, FailedState};
pub use states::{GptPrepState, LinkWaitState, ManifestState, ManifestConfig, ManifestMode};
pub use orchestrator::{download, download_with_config, DownloadResult};
pub use phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
pub use runner::{run_iteration, IterationResult, MainLoopConfig, get_tsc};
