//! Post-EBS state machine: Init → GptPrep → LinkWait → DHCP → DNS → Connect →
//! HTTP → Manifest → Done. Entry point: [`download_with_config`]. Driver init
//! hard-resets the NIC; assumes hwinit has normalized DMA/bus-master/coherency.

pub mod adapter;
pub mod context;
pub mod disk_writer;
pub mod orchestrator;
pub mod serial;
pub mod state;
pub mod states;

pub mod phases;
pub mod runner;

pub use adapter::SmoltcpAdapter;
pub use context::{Context, DownloadConfig, Timeouts};
pub use disk_writer::DiskWriter;
pub use orchestrator::{download, download_with_config, DownloadResult};
pub use phases::{phase1_rx_refill, phase5_tx_completions, TX_BUDGET};
pub use runner::{get_tsc, run_iteration, IterationResult, MainLoopConfig};
pub use serial::{print, print_hex, print_ipv4, print_mac, print_u32, println};
pub use state::{State, StepResult};
pub(crate) use states::{
    ConnectState, DhcpState, DnsState, DoneState, FailedState, GptPrepState, HttpState, InitState,
    LinkWaitState, ManifestState,
};
pub use states::{ManifestConfig, ManifestMode};
