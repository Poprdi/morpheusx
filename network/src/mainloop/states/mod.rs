//! State machine states for the download orchestrator.

pub mod init;
pub mod gpt;
pub mod link;
pub mod dhcp;
pub mod dns;
pub mod connect;
pub mod http;
pub mod done;
pub mod manifest;

pub use init::InitState;
pub use gpt::GptPrepState;
pub use link::LinkWaitState;
pub use dhcp::DhcpState;
pub use dns::DnsState;
pub use connect::ConnectState;
pub use http::HttpState;
pub use done::{DoneState, FailedState};
pub use manifest::{ManifestState, ManifestConfig, ManifestMode};
pub use manifest::{write_manifest_standalone, regenerate_manifest};
