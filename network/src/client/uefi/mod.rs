//! UEFI HTTP client implementation

pub mod client;
pub mod downloader;

pub use client::UefiHttpClient;
pub use downloader::Downloader;
