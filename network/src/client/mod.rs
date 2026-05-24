//! HTTP client implementation.
//!
//! Pure bare-metal HTTP client over TCP/IP. Uses smoltcp for the network
//! stack and dma-pool for DMA memory from code caves in our PE binary.

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
