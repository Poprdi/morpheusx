//! Distro Downloader Module
//!
//! Provides a TUI for browsing, downloading, and managing Linux distribution ISOs.
//! Downloads are stored using chunked FAT32 storage (bypassing 4GB limit).
//! Integrates ISO management for viewing, deleting, and booting downloaded ISOs.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    DistroDownloader                         │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐  │
//! │  │   Catalog   │  │    State     │  │   IsoStorage      │  │
//! │  │  (static)   │  │ (UI + DL)    │  │   (chunked)       │  │
//! │  └─────────────┘  └──────────────┘  └───────────────────┘  │
//! │         │                │                    │            │
//! │         ▼                ▼                    ▼            │
//! │  ┌─────────────────────────────────────────────────────┐   │
//! │  │     UI Event Loop (Browse / Manage / Download)      │   │
//! │  └─────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod catalog;
pub mod state;
pub mod renderer;
pub mod ui;
pub mod manifest_io;

pub use catalog::{DistroCategory, DistroEntry, CATEGORIES, DISTRO_CATALOG, get_by_category};
pub use manifest_io::{persist_manifest, load_manifests_from_esp, delete_manifest, ManifestIoError};
pub use state::{DownloadState, DownloadStatus, UiMode, UiState};
pub use ui::{DistroDownloader, ManageAction};
