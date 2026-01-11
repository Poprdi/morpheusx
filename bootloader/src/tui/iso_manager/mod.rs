//! ISO Manager TUI Module
//!
//! Provides a TUI for managing stored ISO images:
//! - View downloaded ISOs
//! - Delete ISOs (reclaim space)
//! - View ISO details (size, chunks, status)
//! - Boot from ISO
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      IsoManager TUI                         │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐  │
//! │  │   State     │  │   Renderer   │  │    Actions        │  │
//! │  │ (selection) │  │ (list view)  │  │ (boot/delete)     │  │
//! │  └─────────────┘  └──────────────┘  └───────────────────┘  │
//! └─────────────────────────────────────────────────────────────┘
//! ```

mod renderer;
mod state;
mod ui;

pub use state::{IsoManagerState, ViewMode};
pub use ui::IsoManager;
