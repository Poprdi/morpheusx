//! Download commit infrastructure - modular components for UEFI to bare-metal transition.
//!
//! # Architecture
//! - `commit_download` - Main orchestration logic
//! - `pci/` - PCI device probing (NIC, block, config space)
//! - `resources/` - Resource allocation (DMA, stack, handoff)
//! - `uefi/` - UEFI utilities (timing, ESP, helpers)
//! - `display` - UI and display functions

pub mod commit_download;
pub mod display;
pub mod pci;
pub mod resources;
pub mod uefi;

// Re-export main types and functions
pub use commit_download::{commit_to_download, CommitResult};
pub use display::DownloadCommitConfig;
