//! HTTP client implementation.
//!
//! Pure bare-metal HTTP client over TCP/IP. Uses smoltcp for the network
//! stack and dma-pool for DMA memory from code caves in our PE binary.
//!
//! # Example
//!
//! ```ignore
//! use dma_pool::DmaPool;
//! use morpheus_network::client::NativeHttpClient;
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::device::hal::StaticHal;
//! use morpheus_network::stack::NetConfig;
//!
//! // Init DMA from caves
//! unsafe { DmaPool::init_from_caves(image_base, image_end) };
//! StaticHal::init();
//!
//! // Create VirtIO network device
//! let device = VirtioNetDevice::<StaticHal, _>::new(transport)?;
//!
//! // Create native HTTP client
//! let mut client = NativeHttpClient::new(device, NetConfig::Dhcp, get_time_ms);
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

pub mod native;
pub use native::NativeHttpClient;
