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
pub mod commit; // Modular commit download infrastructure
pub mod manifest_io;
pub mod network_check; // Network connectivity verification (deprecated - see commit)
pub mod renderer;
pub mod state;
pub mod ui; // Post-EBS download flow

pub use catalog::{get_by_category, DistroCategory, DistroEntry, CATEGORIES, DISTRO_CATALOG};
pub use commit::{commit_to_download, CommitResult, DownloadCommitConfig};
pub use manifest_io::{
    delete_manifest, load_manifests_from_esp, persist_manifest, ManifestIoError,
};
pub use state::{DownloadState, DownloadStatus, UiMode, UiState};
pub use ui::{DistroDownloader, ManageAction};
