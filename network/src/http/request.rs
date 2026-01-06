//! HTTP request building and serialization.
//!
//! Build HTTP/1.1 requests and serialize them to wire format.
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::http::Request;
//! use morpheus_network::url::Url;
//!
//! let url = Url::parse("http://example.com/api").unwrap();
//! let request = Request::get(url);
//! let wire = request.to_wire_format();
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use crate::types::HttpMethod;
use crate::url::Url;
use super::headers::Headers;

/// HTTP request.
#[derive(Debug, Clone)]
pub struct Request {
    /// HTTP method (GET, POST, etc.).
    pub method: HttpMethod,
    /// Target URL.
    pub url: Url,
    /// Request headers.
    pub headers: Headers,
    /// Optional request body.
    pub body: Option<Vec<u8>>,
}

impl Request {
    /// Create a new request with the given method and URL.
    pub fn new(method: HttpMethod, url: Url) -> Self {
        let mut headers = Headers::new();
        
        // Set default headers
        headers.set_host(url.host_header());
        headers.set("User-Agent", "MorpheusX/1.0");
        headers.set("Accept", "*/*");
        headers.set("Connection", "close");
        
        Self {
            method,
            url,
            headers,
            body: None,
        }
    }

    /// Create a GET request.
    pub fn get(url: Url) -> Self {
        Self::new(HttpMethod::Get, url)
    }

    /// Create a HEAD request (for checking file size, etc.).
    pub fn head(url: Url) -> Self {
        Self::new(HttpMethod::Head, url)
    }

    /// Create a POST request.
    pub fn post(url: Url) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    /// Create a PUT request.
    pub fn put(url: Url) -> Self {
        Self::new(HttpMethod::Put, url)
    }

    /// Create a DELETE request.
    pub fn delete(url: Url) -> Self {
        Self::new(HttpMethod::Delete, url)
    }

    /// Set the request body.
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.headers.set_content_length(body.len());
        self.body = Some(body);
        self
    }

    /// Set a header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Set Content-Type header.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.headers.set_content_type(content_type);
        self
    }

    /// Get the request method as string.
    pub fn method_str(&self) -> &'static str {
        match self.method {
            HttpMethod::Get => "GET",
            HttpMethod::Head => "HEAD",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
        }
    }

    /// Serialize the request to HTTP/1.1 wire format.
    ///
    /// Format:
    /// ```text
    /// METHOD /path HTTP/1.1\r\n
    /// Header: Value\r\n
    /// ...
    /// \r\n
    /// [body]
    /// ```
    pub fn to_wire_format(&self) -> Vec<u8> {
        let mut result = String::new();
        
        // Request line: METHOD /path HTTP/1.1
        result.push_str(self.method_str());
        result.push(' ');
        result.push_str(&self.url.request_uri());
        result.push_str(" HTTP/1.1\r\n");
        
        // Headers
        result.push_str(&self.headers.to_wire_format());
        
        // Empty line to end headers
        result.push_str("\r\n");
        
        // Convert to bytes and append body if present
        let mut bytes = result.into_bytes();
        if let Some(ref body) = self.body {
            bytes.extend_from_slice(body);
        }
        
        bytes
    }

    /// Get the total size of the serialized request.
    pub fn wire_size(&self) -> usize {
        let body_len = self.body.as_ref().map(|b| b.len()).unwrap_or(0);
        self.to_wire_format().len() - body_len + body_len // Could optimize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::Url;

    fn test_url() -> Url {
        Url::parse("http://example.com/api/test").unwrap()
    }

    // ==================== Request Creation ====================

    #[test]
    fn test_new_request() {
        let request = Request::new(HttpMethod::Get, test_url());
        assert!(matches!(request.method, HttpMethod::Get));
        assert_eq!(request.url.host, "example.com");
    }

    #[test]
    fn test_get_request() {
        let request = Request::get(test_url());
        assert!(matches!(request.method, HttpMethod::Get));
    }

    #[test]
    fn test_head_request() {
        let request = Request::head(test_url());
        assert!(matches!(request.method, HttpMethod::Head));
    }

    #[test]
    fn test_post_request() {
        let request = Request::post(test_url());
        assert!(matches!(request.method, HttpMethod::Post));
    }

    #[test]
    fn test_put_request() {
        let request = Request::put(test_url());
        assert!(matches!(request.method, HttpMethod::Put));
    }

    #[test]
    fn test_delete_request() {
        let request = Request::delete(test_url());
        assert!(matches!(request.method, HttpMethod::Delete));
    }

    // ==================== Default Headers ====================

    #[test]
    fn test_default_host_header() {
        let request = Request::get(test_url());
        assert_eq!(request.headers.host(), Some("example.com"));
    }

    #[test]
    fn test_default_host_header_with_port() {
        let url = Url::parse("http://localhost:8080/api").unwrap();
        let request = Request::get(url);
        assert_eq!(request.headers.host(), Some("localhost:8080"));
    }

    #[test]
    fn test_default_user_agent() {
        let request = Request::get(test_url());
        assert_eq!(request.headers.get("User-Agent"), Some("MorpheusX/1.0"));
    }

    #[test]
    fn test_default_accept() {
        let request = Request::get(test_url());
        assert_eq!(request.headers.get("Accept"), Some("*/*"));
    }

    #[test]
    fn test_default_connection() {
        let request = Request::get(test_url());
        assert_eq!(request.headers.get("Connection"), Some("close"));
    }

    // ==================== Builder Methods ====================

    #[test]
    fn test_with_body() {
        let body = b"Hello, World!".to_vec();
        let request = Request::post(test_url()).with_body(body.clone());
        
        assert_eq!(request.body, Some(body));
        assert_eq!(request.headers.content_length(), Some(13));
    }

    #[test]
    fn test_with_header() {
        let request = Request::get(test_url())
            .with_header("X-Custom", "value");
        
        assert_eq!(request.headers.get("X-Custom"), Some("value"));
    }

    #[test]
    fn test_with_content_type() {
        let request = Request::post(test_url())
            .with_content_type("application/json");
        
        assert_eq!(request.headers.content_type(), Some("application/json"));
    }

    // ==================== Method String ====================

    #[test]
    fn test_method_str_get() {
        let request = Request::get(test_url());
        assert_eq!(request.method_str(), "GET");
    }

    #[test]
    fn test_method_str_post() {
        let request = Request::post(test_url());
        assert_eq!(request.method_str(), "POST");
    }

    // ==================== Wire Format ====================

    #[test]
    fn test_to_wire_format_request_line() {
        let request = Request::get(test_url());
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.starts_with("GET /api/test HTTP/1.1\r\n"));
    }

    #[test]
    fn test_to_wire_format_headers() {
        let request = Request::get(test_url());
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.contains("Host: example.com\r\n"));
        assert!(wire.contains("User-Agent: MorpheusX/1.0\r\n"));
    }

    #[test]
    fn test_to_wire_format_ends_with_double_crlf() {
        let request = Request::get(test_url());
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_to_wire_format_with_body() {
        let body = b"test body".to_vec();
        let request = Request::post(test_url()).with_body(body);
        let wire = request.to_wire_format();
        
        // Should end with body, not \r\n\r\n
        assert!(wire.ends_with(b"test body"));
        
        // Should contain Content-Length
        let wire_str = String::from_utf8_lossy(&wire);
        assert!(wire_str.contains("Content-Length: 9\r\n"));
    }

    #[test]
    fn test_to_wire_format_with_query() {
        let url = Url::parse("http://example.com/search?q=rust").unwrap();
        let request = Request::get(url);
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.starts_with("GET /search?q=rust HTTP/1.1\r\n"));
    }

    // ==================== Real-World Requests ====================

    #[test]
    fn test_iso_download_request() {
        let url = Url::parse(
            "http://releases.ubuntu.com/24.04/ubuntu-24.04-live-server-amd64.iso"
        ).unwrap();
        let request = Request::get(url);
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.starts_with("GET /24.04/ubuntu-24.04-live-server-amd64.iso HTTP/1.1\r\n"));
        assert!(wire.contains("Host: releases.ubuntu.com\r\n"));
    }

    #[test]
    fn test_head_request_for_size() {
        let url = Url::parse("http://mirror.example.com/file.iso").unwrap();
        let request = Request::head(url);
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.starts_with("HEAD /file.iso HTTP/1.1\r\n"));
    }

    #[test]
    fn test_api_post_request() {
        let url = Url::parse("https://api.example.com/v1/data").unwrap();
        let body = br#"{"key": "value"}"#.to_vec();
        let request = Request::post(url)
            .with_content_type("application/json")
            .with_body(body);
        
        let wire = String::from_utf8(request.to_wire_format()).unwrap();
        
        assert!(wire.starts_with("POST /v1/data HTTP/1.1\r\n"));
        assert!(wire.contains("Content-Type: application/json\r\n"));
        assert!(wire.contains("Content-Length: 16\r\n"));
        assert!(wire.ends_with(r#"{"key": "value"}"#));
    }
}
