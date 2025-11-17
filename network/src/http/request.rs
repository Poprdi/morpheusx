//! HTTP request

use alloc::vec::Vec;
use crate::types::HttpMethod;
use crate::url::Url;
use super::headers::Headers;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}

impl Request {
    pub fn new(method: HttpMethod, url: Url) -> Self {
        Self {
            method,
            url,
            headers: Headers::new(),
            body: None,
        }
    }

    // TODO: Implement request builders (get, post, head, etc.)
    // TODO: Add header manipulation methods
    // TODO: Serialize to wire format
}
