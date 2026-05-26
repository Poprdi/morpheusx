//! HTTP/1.1 request builder.

use super::headers::Headers;
use crate::types::HttpMethod;
use crate::url::Url;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}

impl Request {
    /// Seeds Host, User-Agent, Accept, Connection: close.
    pub fn new(method: HttpMethod, url: Url) -> Self {
        let mut headers = Headers::new();

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

    pub fn get(url: Url) -> Self {
        Self::new(HttpMethod::Get, url)
    }

    pub fn head(url: Url) -> Self {
        Self::new(HttpMethod::Head, url)
    }

    pub fn post(url: Url) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    pub fn put(url: Url) -> Self {
        Self::new(HttpMethod::Put, url)
    }

    pub fn delete(url: Url) -> Self {
        Self::new(HttpMethod::Delete, url)
    }

    /// Sets Content-Length automatically.
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.headers.set_content_length(body.len());
        self.body = Some(body);
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.set(name, value);
        self
    }

    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.headers.set_content_type(content_type);
        self
    }

    pub fn method_str(&self) -> &'static str {
        self.method.as_str()
    }

    /// `METHOD path HTTP/1.1\r\n<headers>\r\n[body]`.
    pub fn to_wire_format(&self) -> Vec<u8> {
        let mut result = String::new();

        result.push_str(self.method_str());
        result.push(' ');
        result.push_str(&self.url.request_uri());
        result.push_str(" HTTP/1.1\r\n");

        result.push_str(&self.headers.to_wire_format());
        result.push_str("\r\n");

        let mut bytes = result.into_bytes();
        if let Some(ref body) = self.body {
            bytes.extend_from_slice(body);
        }

        bytes
    }

    pub fn wire_size(&self) -> usize {
        let body_len = self.body.as_ref().map(|b| b.len()).unwrap_or(0);
        self.to_wire_format().len() - body_len + body_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::Url;

    fn test_url() -> Url {
        Url::parse("http://example.com/api/test").unwrap()
    }


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


    #[test]
    fn test_with_body() {
        let body = b"Hello, World!".to_vec();
        let request = Request::post(test_url()).with_body(body.clone());

        assert_eq!(request.body, Some(body));
        assert_eq!(request.headers.content_length(), Some(13));
    }

    #[test]
    fn test_with_header() {
        let request = Request::get(test_url()).with_header("X-Custom", "value");

        assert_eq!(request.headers.get("X-Custom"), Some("value"));
    }

    #[test]
    fn test_with_content_type() {
        let request = Request::post(test_url()).with_content_type("application/json");

        assert_eq!(request.headers.content_type(), Some("application/json"));
    }


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


    #[test]
    fn test_iso_download_request() {
        let url = Url::parse("http://releases.ubuntu.com/24.04/ubuntu-24.04-live-server-amd64.iso")
            .unwrap();
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
