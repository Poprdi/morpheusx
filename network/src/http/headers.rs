//! HTTP headers

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct Headers {
    headers: Vec<Header>,
}

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: add() - Add header
    // TODO: get() - Get header (case-insensitive)
    // TODO: remove() - Remove header
    // TODO: Parse from wire format "Name: Value\r\n"
    // TODO: Serialize to wire format
    // TODO: content_length() helper
    // TODO: is_chunked() helper
}
