//! Distro Downloader Module
//!
//! Provides a TUI for browsing and downloading Linux distribution ISOs.
//! Downloads are saved to the ESP partition in `/isos/` directory.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    DistroDownloader                         │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐  │
//! │  │   Catalog   │  │    State     │  │     Renderer      │  │
//! │  │  (static)   │  │ (UI + DL)    │  │   (TUI output)    │  │
//! │  └─────────────┘  └──────────────┘  └───────────────────┘  │
//! │         │                │                    │            │
//! │         ▼                ▼                    ▼            │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │              UI Event Loop (run)                    │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod catalog;
pub mod state;
pub mod renderer;
pub mod ui;

pub use catalog::{DistroCategory, DistroEntry, CATEGORIES, DISTRO_CATALOG, get_by_category};
pub use state::{DownloadState, DownloadStatus, UiMode, UiState};
pub use ui::DistroDownloader;
