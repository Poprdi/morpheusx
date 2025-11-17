//! HTTP client interface and implementations

use crate::error::Result;
use crate::http::{Request, Response};
use crate::types::ProgressCallback;

/// HTTP client trait
pub trait HttpClient {
    /// Execute an HTTP request
    fn request(&mut self, request: &Request) -> Result<Response>;

    /// Execute request with progress tracking
    fn request_with_progress(
        &mut self,
        request: &Request,
        progress: ProgressCallback,
    ) -> Result<Response>;

    /// Check if client is ready
    fn is_ready(&self) -> bool;
}

#[cfg(target_os = "uefi")]
pub mod uefi;
