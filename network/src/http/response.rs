//! HTTP response parsing.
//!
//! Parse HTTP/1.1 responses from wire format.
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::http::Response;
//!
//! let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello";
//! let response = Response::parse(data).unwrap();
//! assert_eq!(response.status_code, 200);
//! ```

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::error::{NetworkError, Result};
use super::headers::Headers;

/// HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP version (e.g., "HTTP/1.1").
    pub version: String,
    /// Status code (e.g., 200, 404).
    pub status_code: u16,
    /// Reason phrase (e.g., "OK", "Not Found").
    pub reason: String,
    /// Response headers.
    pub headers: Headers,
    /// Response body.
    pub body: Vec<u8>,
}

impl Response {
    /// Create a new response with the given status code.
    pub fn new(status_code: u16) -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status_code,
            reason: Self::default_reason(status_code).to_string(),
            headers: Headers::new(),
            body: Vec::new(),
        }
    }

    /// Check if response indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    /// Check if response indicates redirect (3xx).
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status_code)
    }

    /// Check if response indicates client error (4xx).
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status_code)
    }

    /// Check if response indicates server error (5xx).
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status_code)
    }

    /// Get default reason phrase for status code.
    pub fn default_reason(status_code: u16) -> &'static str {
        match status_code {
            100 => "Continue",
            101 => "Switching Protocols",
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            206 => "Partial Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            408 => "Request Timeout",
            416 => "Range Not Satisfiable",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            _ => "Unknown",
        }
    }

    /// Get Location header (for redirects).
    pub fn location(&self) -> Option<&str> {
        self.headers.get("Location")
    }

    /// Get Content-Length.
    pub fn content_length(&self) -> Option<usize> {
        self.headers.content_length()
    }

    /// Check if response has chunked transfer encoding.
    pub fn is_chunked(&self) -> bool {
        self.headers.is_chunked()
    }

    /// Get Content-Type.
    pub fn content_type(&self) -> Option<&str> {
        self.headers.content_type()
    }

    // ==================== Parsing ====================

    /// Parse a complete HTTP response from raw bytes.
    ///
    /// Returns the parsed response and the total bytes consumed.
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        // Convert to string for easier parsing (HTTP/1.1 is ASCII)
        let text = core::str::from_utf8(data).map_err(|_| NetworkError::InvalidResponse)?;
        
        // Find end of headers
        let headers_end = text.find("\r\n\r\n").ok_or(NetworkError::InvalidResponse)?;
        
        // Parse status line (first line)
        let first_line_end = text.find("\r\n").ok_or(NetworkError::InvalidResponse)?;
        let status_line = &text[..first_line_end];
        let (version, status_code, reason) = Self::parse_status_line(status_line)?;
        
        // Parse headers (after first line, if any exist before headers_end)
        let headers_start = first_line_end + 2;
        let headers = if headers_start < headers_end {
            let (h, _) = Headers::from_wire_format(&text[headers_start..headers_end]);
            h
        } else {
            Headers::new()
        };
        
        // Calculate body start
        let body_start = headers_end + 4; // Skip \r\n\r\n
        
        // Get body based on Content-Length or rest of data
        let body_len = headers.content_length().unwrap_or(data.len() - body_start);
        let body_end = body_start + body_len.min(data.len() - body_start);
        let body = data[body_start..body_end].to_vec();
        
        let response = Self {
            version,
            status_code,
            reason,
            headers,
            body,
        };
        
        Ok((response, body_end))
    }

    /// Parse status line: "HTTP/1.1 200 OK"
    fn parse_status_line(line: &str) -> Result<(String, u16, String)> {
        let mut parts = line.splitn(3, ' ');
        
        let version = parts.next().ok_or(NetworkError::InvalidResponse)?;
        if !version.starts_with("HTTP/") {
            return Err(NetworkError::InvalidResponse);
        }
        
        let status_str = parts.next().ok_or(NetworkError::InvalidResponse)?;
        let status_code = status_str.parse::<u16>().map_err(|_| NetworkError::InvalidResponse)?;
        
        let reason = parts.next().unwrap_or("").to_string();
        
        Ok((version.to_string(), status_code, reason))
    }

    /// Parse headers only, without body.
    ///
    /// Useful for HEAD responses or when body will be streamed separately.
    pub fn parse_headers_only(data: &[u8]) -> Result<(Self, usize)> {
        let text = core::str::from_utf8(data).map_err(|_| NetworkError::InvalidResponse)?;
        
        let headers_end = text.find("\r\n\r\n").ok_or(NetworkError::InvalidResponse)?;
        
        let first_line_end = text.find("\r\n").ok_or(NetworkError::InvalidResponse)?;
        let status_line = &text[..first_line_end];
        let (version, status_code, reason) = Self::parse_status_line(status_line)?;
        
        let headers_start = first_line_end + 2;
        let headers = if headers_start < headers_end {
            let (h, _) = Headers::from_wire_format(&text[headers_start..headers_end]);
            h
        } else {
            Headers::new()
        };
        
        let response = Self {
            version,
            status_code,
            reason,
            headers,
            body: Vec::new(),
        };
        
        // Return byte offset where body would start
        Ok((response, headers_end + 4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Response Creation ====================

    #[test]
    fn test_new_response() {
        let response = Response::new(200);
        assert_eq!(response.status_code, 200);
        assert_eq!(response.reason, "OK");
        assert_eq!(response.version, "HTTP/1.1");
    }

    #[test]
    fn test_new_response_404() {
        let response = Response::new(404);
        assert_eq!(response.status_code, 404);
        assert_eq!(response.reason, "Not Found");
    }

    // ==================== Status Checks ====================

    #[test]
    fn test_is_success() {
        assert!(Response::new(200).is_success());
        assert!(Response::new(201).is_success());
        assert!(Response::new(204).is_success());
        assert!(Response::new(299).is_success());
        assert!(!Response::new(300).is_success());
        assert!(!Response::new(404).is_success());
    }

    #[test]
    fn test_is_redirect() {
        assert!(Response::new(301).is_redirect());
        assert!(Response::new(302).is_redirect());
        assert!(Response::new(307).is_redirect());
        assert!(!Response::new(200).is_redirect());
        assert!(!Response::new(404).is_redirect());
    }

    #[test]
    fn test_is_client_error() {
        assert!(Response::new(400).is_client_error());
        assert!(Response::new(404).is_client_error());
        assert!(Response::new(499).is_client_error());
        assert!(!Response::new(200).is_client_error());
        assert!(!Response::new(500).is_client_error());
    }

    #[test]
    fn test_is_server_error() {
        assert!(Response::new(500).is_server_error());
        assert!(Response::new(503).is_server_error());
        assert!(!Response::new(200).is_server_error());
        assert!(!Response::new(404).is_server_error());
    }

    // ==================== Default Reasons ====================

    #[test]
    fn test_default_reason() {
        assert_eq!(Response::default_reason(200), "OK");
        assert_eq!(Response::default_reason(404), "Not Found");
        assert_eq!(Response::default_reason(500), "Internal Server Error");
        assert_eq!(Response::default_reason(999), "Unknown");
    }

    // ==================== Basic Parsing ====================

    #[test]
    fn test_parse_simple_response() {
        let data = b"HTTP/1.1 200 OK\r\n\r\n";
        let (response, consumed) = Response::parse(data).unwrap();
        
        assert_eq!(response.version, "HTTP/1.1");
        assert_eq!(response.status_code, 200);
        assert_eq!(response.reason, "OK");
        assert_eq!(consumed, data.len());
    }

    #[test]
    fn test_parse_response_with_headers() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nHello";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_type(), Some("text/html"));
        assert_eq!(response.content_length(), Some(5));
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn test_parse_response_with_body() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nHello World";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.body, b"Hello World");
    }

    #[test]
    fn test_parse_404_response() {
        let data = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.status_code, 404);
        assert_eq!(response.reason, "Not Found");
        assert!(!response.is_success());
    }

    // ==================== Redirect Responses ====================

    #[test]
    fn test_parse_redirect_response() {
        let data = b"HTTP/1.1 302 Found\r\nLocation: http://example.com/new\r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert!(response.is_redirect());
        assert_eq!(response.location(), Some("http://example.com/new"));
    }

    #[test]
    fn test_parse_301_redirect() {
        let data = b"HTTP/1.1 301 Moved Permanently\r\nLocation: https://secure.example.com/\r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.status_code, 301);
        assert!(response.is_redirect());
        assert_eq!(response.location(), Some("https://secure.example.com/"));
    }

    // ==================== Chunked Responses ====================

    #[test]
    fn test_parse_chunked_response() {
        let data = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert!(response.is_chunked());
        assert_eq!(response.content_length(), None);
    }

    // ==================== Headers Only ====================

    #[test]
    fn test_parse_headers_only() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nBody starts here...";
        let (response, body_offset) = Response::parse_headers_only(data).unwrap();
        
        assert_eq!(response.status_code, 200);
        assert_eq!(response.content_length(), Some(1000));
        assert!(response.body.is_empty()); // Body not parsed
        // 17 (status line) + 22 (header) + 2 (blank line) = 41 bytes
        assert_eq!(body_offset, 41); // Where body starts
    }

    // ==================== Error Cases ====================

    #[test]
    fn test_parse_invalid_no_headers_end() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Type: text/html";
        assert!(Response::parse(data).is_err());
    }

    #[test]
    fn test_parse_invalid_status_code() {
        let data = b"HTTP/1.1 ABC OK\r\n\r\n";
        assert!(Response::parse(data).is_err());
    }

    #[test]
    fn test_parse_invalid_version() {
        let data = b"HTTT/1.1 200 OK\r\n\r\n";
        assert!(Response::parse(data).is_err());
    }

    #[test]
    fn test_parse_empty_data() {
        let data = b"";
        assert!(Response::parse(data).is_err());
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_parse_no_reason_phrase() {
        let data = b"HTTP/1.1 200 \r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.status_code, 200);
        assert_eq!(response.reason, "");
    }

    #[test]
    fn test_parse_http_10() {
        let data = b"HTTP/1.0 200 OK\r\n\r\n";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.version, "HTTP/1.0");
    }

    #[test]
    fn test_parse_partial_body() {
        // Body shorter than Content-Length
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nShort";
        let (response, _) = Response::parse(data).unwrap();
        
        assert_eq!(response.body, b"Short");
    }

    // ==================== Real-World Responses ====================

    #[test]
    fn test_parse_iso_download_response() {
        let data = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Length: 1048576000\r\n",
            "Accept-Ranges: bytes\r\n",
            "Connection: close\r\n",
            "\r\n"
        );
        let (response, body_offset) = Response::parse_headers_only(data.as_bytes()).unwrap();
        
        assert!(response.is_success());
        assert_eq!(response.content_type(), Some("application/octet-stream"));
        assert_eq!(response.content_length(), Some(1048576000)); // ~1GB
        assert_eq!(response.headers.get("Accept-Ranges"), Some("bytes"));
        assert_eq!(body_offset, data.len());
    }

    #[test]
    fn test_parse_head_response() {
        let data = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/x-iso9660-image\r\n",
            "Content-Length: 3221225472\r\n",
            "\r\n"
        );
        let (response, _) = Response::parse(data.as_bytes()).unwrap();
        
        assert!(response.is_success());
        assert_eq!(response.content_length(), Some(3221225472)); // 3GB
        assert!(response.body.is_empty()); // HEAD has no body
    }
}
