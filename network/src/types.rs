//! Common type definitions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Delete,
}

/// Progress callback: (bytes_transferred, total_bytes_if_known)
pub type ProgressCallback = fn(usize, Option<usize>);
