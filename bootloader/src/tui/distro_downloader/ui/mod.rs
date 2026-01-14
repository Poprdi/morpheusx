//! Distro Downloader UI Module
//!
//! Main TUI for browsing, downloading, and managing Linux distributions.
//! Integrates ISO storage management for chunked downloads.
//!
//! # Architecture
//!
//! ```text
//! ui/
//! ├── mod.rs        # This file - module declarations and re-exports
//! ├── controller.rs # DistroDownloader struct and core logic (~200 LOC)
//! ├── input.rs      # Input handling methods (~200 LOC)
//! ├── render.rs     # All rendering methods (~400 LOC)
//! └── helpers.rs    # Helper functions and constants (~50 LOC)
//! ```
//!
//! # Rendering Pattern
//!
//! Follows the same pattern as main_menu and distro_launcher:
//! - Clear screen once at start
//! - Render initial state
//! - Only re-render after handling input (no clear in render loop)

mod controller;
mod helpers;
mod input;
mod render;

pub use controller::DistroDownloader;
pub use helpers::ManageAction;
