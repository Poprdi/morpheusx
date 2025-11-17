//! URL parser

use alloc::string::String;
use crate::error::{NetworkError, Result};

/// Parsed URL
#[derive(Debug, Clone)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: Option<String>,
}

impl Url {
    /// Parse a URL string
    pub fn parse(_url: &str) -> Result<Self> {
        // TODO: Implement URL parsing
        // 1. Extract scheme (http/https)
        // 2. Extract host and optional port
        // 3. Extract path (default to "/")
        // 4. Extract query string
        // 5. Validate components
        Err(NetworkError::InvalidUrl)
    }

    /// Get port with default for scheme
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(80) // TODO: Use 443 for https
    }

    /// Check if HTTPS
    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }
}
