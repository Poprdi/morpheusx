//! Bare-metal HTTP client over smoltcp; DMA from PE code caves via dma-pool.

use crate::error::Result;
use crate::http::{Request, Response};
use crate::types::ProgressCallback;

pub trait HttpClient {
    fn request(&mut self, request: &Request) -> Result<Response>;

    fn request_with_progress(
        &mut self,
        request: &Request,
        progress: ProgressCallback,
    ) -> Result<Response>;

    fn is_ready(&self) -> bool;
}

pub mod native;
pub use native::NativeHttpClient;
