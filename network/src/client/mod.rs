//! HTTP client interface and implementations.
//!
//! This module provides two HTTP client implementations:
//!
//! - [`uefi::UefiHttpClient`] - Uses UEFI HTTP Boot protocol (requires firmware support)
//! - [`native::NativeHttpClient`] - Bare metal TCP/IP over any `NetworkDevice`
//!
//! # Choosing a Client
//!
//! Use **`NativeHttpClient`** (preferred):
//! - Works with any network hardware via drivers
//! - No UEFI firmware dependencies
//! - Full control over network stack
//! - Works in QEMU with virtio-net
//!
//! Use **`UefiHttpClient`** when:
//! - UEFI firmware has HTTP Boot support
//! - You need HTTPS (TLS in firmware)
//! - Simpler setup (firmware handles everything)
//!
//! # Example
//!
//! ```ignore
//! use morpheus_network::client::native::NativeHttpClient;
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::stack::NetConfig;
//!
//! // Create VirtIO network device
//! let device = VirtioNetDevice::new(transport)?;
//!
//! // Create native HTTP client
//! let mut client = NativeHttpClient::new(device, NetConfig::dhcp(), get_time_ms);
//!
//! // Wait for DHCP
//! client.wait_for_network(30_000)?;
//!
//! // Download file
//! let response = client.get("http://mirror.example.com/tails.iso")?;
//! ```

use crate::error::Result;
use crate::http::{Request, Response};
use crate::types::ProgressCallback;

/// HTTP client trait.
///
/// Implemented by both UEFI and native clients for interchangeable use.
pub trait HttpClient {
    /// Execute an HTTP request.
    fn request(&mut self, request: &Request) -> Result<Response>;

    /// Execute request with progress tracking.
    fn request_with_progress(
        &mut self,
        request: &Request,
        progress: ProgressCallback,
    ) -> Result<Response>;

    /// Check if client is ready to make requests.
    fn is_ready(&self) -> bool;
}

// Native bare-metal client (always available)
pub mod native;
pub use native::NativeHttpClient;

// UEFI protocol-based client (UEFI only)
#[cfg(target_os = "uefi")]
pub mod uefi;

#[cfg(target_os = "uefi")]
pub use uefi::UefiHttpClient;
