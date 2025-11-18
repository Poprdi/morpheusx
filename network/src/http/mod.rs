//! HTTP message handling
//!
//! TODO: Implement HTTP protocol
//! - Request building (GET, POST, HEAD, etc.)
//! - Response parsing
//! - Header management
//! - Status codes
//! - Message formatting

pub mod request;
pub mod response;
pub mod headers;

pub use request::Request;
pub use response::Response;
pub use headers::Headers;