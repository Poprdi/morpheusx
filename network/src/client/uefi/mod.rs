//! UEFI HTTP client implementation.
//!
//! Provides HTTP client functionality for UEFI environments:
//! - `UefiHttpClient` - Main HTTP client using UEFI protocols
//! - `Downloader` - High-level download manager
//! - `DownloadBuilder` - Fluent API for downloads

pub mod client;
pub mod downloader;

pub use client::{UefiHttpClient, ClientConfig};
pub use downloader::{Downloader, DownloadBuilder, DownloadConfig, DownloadResult};
