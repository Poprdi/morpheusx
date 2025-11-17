//! URL parsing and manipulation
//!
//! TODO: Implement URL parser
//! - Parse scheme://host[:port]/path[?query][#fragment]
//! - Validate HTTP/HTTPS schemes
//! - Extract components
//! - Handle URL encoding/decoding

pub mod parser;

pub use parser::Url;
