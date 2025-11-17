//! HTTP response

use alloc::vec::Vec;
use super::headers::Headers;

#[derive(Debug, Clone)]
pub struct Response {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status_code: u16) -> Self {
        Self {
            status_code,
            headers: Headers::new(),
            body: Vec::new(),
        }
    }

    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 300
    }

    // TODO: Parse response from wire format
    // TODO: Handle status line parsing
    // TODO: Content-Length handling
}
