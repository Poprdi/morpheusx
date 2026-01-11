//! HTTP header management.
//!
//! Case-insensitive header storage and retrieval for HTTP requests and responses.
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::http::Headers;
//!
//! let mut headers = Headers::new();
//! headers.set("Content-Type", "application/json");
//! assert_eq!(headers.get("content-type"), Some("application/json"));
//! ```

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A single HTTP header (name-value pair).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Header name (stored in original case).
    pub name: String,
    /// Header value.
    pub value: String,
}

impl Header {
    /// Create a new header.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Check if this header matches the given name (case-insensitive).
    pub fn name_matches(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

/// Collection of HTTP headers.
///
/// Headers are stored in insertion order and support case-insensitive lookup.
/// Multiple headers with the same name are allowed (e.g., Set-Cookie).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    headers: Vec<Header>,
}

impl Headers {
    /// Create an empty header collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of headers.
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// Add a header (allows duplicates).
    ///
    /// Use this when multiple values for the same header are valid
    /// (e.g., Set-Cookie).
    pub fn add(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.headers.push(Header::new(name, value));
    }

    /// Set a header (replaces existing with same name).
    ///
    /// Use this for headers that should have only one value
    /// (e.g., Content-Type, Host).
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let value = value.into();

        // Remove existing headers with same name
        self.headers.retain(|h| !h.name_matches(&name));
        self.headers.push(Header::new(name, value));
    }

    /// Get the first header value by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name_matches(name))
            .map(|h| h.value.as_str())
    }

    /// Get all header values by name (case-insensitive).
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|h| h.name_matches(name))
            .map(|h| h.value.as_str())
            .collect()
    }

    /// Check if a header exists (case-insensitive).
    pub fn contains(&self, name: &str) -> bool {
        self.headers.iter().any(|h| h.name_matches(name))
    }

    /// Remove all headers with the given name (case-insensitive).
    ///
    /// Returns the number of headers removed.
    pub fn remove(&mut self, name: &str) -> usize {
        let before = self.headers.len();
        self.headers.retain(|h| !h.name_matches(name));
        before - self.headers.len()
    }

    /// Iterate over all headers.
    pub fn iter(&self) -> impl Iterator<Item = &Header> {
        self.headers.iter()
    }

    /// Clear all headers.
    pub fn clear(&mut self) {
        self.headers.clear();
    }

    // ==================== Common Header Helpers ====================

    /// Get Content-Length header as usize.
    pub fn content_length(&self) -> Option<usize> {
        self.get("Content-Length").and_then(|v| v.parse().ok())
    }

    /// Set Content-Length header.
    pub fn set_content_length(&mut self, length: usize) {
        self.set("Content-Length", alloc::format!("{}", length));
    }

    /// Get Content-Type header.
    pub fn content_type(&self) -> Option<&str> {
        self.get("Content-Type")
    }

    /// Set Content-Type header.
    pub fn set_content_type(&mut self, content_type: impl Into<String>) {
        self.set("Content-Type", content_type);
    }

    /// Check if Transfer-Encoding is chunked.
    pub fn is_chunked(&self) -> bool {
        self.get("Transfer-Encoding")
            .map(|v| v.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false)
    }

    /// Get Host header.
    pub fn host(&self) -> Option<&str> {
        self.get("Host")
    }

    /// Set Host header.
    pub fn set_host(&mut self, host: impl Into<String>) {
        self.set("Host", host);
    }

    /// Get Connection header.
    pub fn connection(&self) -> Option<&str> {
        self.get("Connection")
    }

    /// Check if connection should be kept alive.
    pub fn keep_alive(&self) -> bool {
        self.get("Connection")
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(false)
    }

    // ==================== Serialization ====================

    /// Serialize headers to wire format (for HTTP requests).
    ///
    /// Each header is formatted as: `Name: Value\r\n`
    pub fn to_wire_format(&self) -> String {
        let mut result = String::new();
        for header in &self.headers {
            result.push_str(&header.name);
            result.push_str(": ");
            result.push_str(&header.value);
            result.push_str("\r\n");
        }
        result
    }

    /// Parse headers from wire format.
    ///
    /// Expects format: `Name: Value\r\n` for each header.
    /// Stops at empty line (double CRLF).
    pub fn from_wire_format(data: &str) -> (Self, usize) {
        let mut headers = Headers::new();
        let mut consumed = 0;

        for line in data.split("\r\n") {
            consumed += line.len() + 2; // Include \r\n

            if line.is_empty() {
                // Empty line marks end of headers
                break;
            }

            if let Some(colon_pos) = line.find(':') {
                let name = line[..colon_pos].trim();
                let value = line[colon_pos + 1..].trim();
                if !name.is_empty() {
                    headers.add(name, value);
                }
            }
            // Invalid lines are silently ignored (common in HTTP)
        }

        (headers, consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // ==================== Basic Operations ====================

    #[test]
    fn test_new_headers_empty() {
        let headers = Headers::new();
        assert!(headers.is_empty());
        assert_eq!(headers.len(), 0);
    }

    #[test]
    fn test_add_header() {
        let mut headers = Headers::new();
        headers.add("Content-Type", "text/html");
        assert_eq!(headers.len(), 1);
        assert!(!headers.is_empty());
    }

    #[test]
    fn test_get_header() {
        let mut headers = Headers::new();
        headers.add("Content-Type", "application/json");
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
    }

    #[test]
    fn test_get_nonexistent_header() {
        let headers = Headers::new();
        assert_eq!(headers.get("Content-Type"), None);
    }

    // ==================== Case Insensitivity ====================

    #[test]
    fn test_get_case_insensitive() {
        let mut headers = Headers::new();
        headers.add("Content-Type", "text/plain");

        assert_eq!(headers.get("content-type"), Some("text/plain"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain"));
        assert_eq!(headers.get("Content-TYPE"), Some("text/plain"));
    }

    #[test]
    fn test_contains_case_insensitive() {
        let mut headers = Headers::new();
        headers.add("Host", "example.com");

        assert!(headers.contains("host"));
        assert!(headers.contains("HOST"));
        assert!(headers.contains("Host"));
    }

    // ==================== Set vs Add ====================

    #[test]
    fn test_add_allows_duplicates() {
        let mut headers = Headers::new();
        headers.add("Set-Cookie", "session=abc");
        headers.add("Set-Cookie", "user=123");

        assert_eq!(headers.len(), 2);
        let cookies = headers.get_all("Set-Cookie");
        assert_eq!(cookies, vec!["session=abc", "user=123"]);
    }

    #[test]
    fn test_set_replaces_existing() {
        let mut headers = Headers::new();
        headers.set("Content-Type", "text/html");
        headers.set("Content-Type", "application/json");

        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
    }

    #[test]
    fn test_set_replaces_case_insensitive() {
        let mut headers = Headers::new();
        headers.set("content-type", "text/html");
        headers.set("Content-Type", "application/json");

        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("content-type"), Some("application/json"));
    }

    // ==================== Remove ====================

    #[test]
    fn test_remove_header() {
        let mut headers = Headers::new();
        headers.add("Content-Type", "text/html");
        headers.add("Host", "example.com");

        let removed = headers.remove("Content-Type");
        assert_eq!(removed, 1);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("Content-Type"), None);
    }

    #[test]
    fn test_remove_multiple() {
        let mut headers = Headers::new();
        headers.add("Set-Cookie", "a=1");
        headers.add("Set-Cookie", "b=2");
        headers.add("Host", "example.com");

        let removed = headers.remove("Set-Cookie");
        assert_eq!(removed, 2);
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut headers = Headers::new();
        headers.add("Host", "example.com");

        let removed = headers.remove("Content-Type");
        assert_eq!(removed, 0);
        assert_eq!(headers.len(), 1);
    }

    // ==================== Content-Length ====================

    #[test]
    fn test_content_length() {
        let mut headers = Headers::new();
        headers.add("Content-Length", "12345");
        assert_eq!(headers.content_length(), Some(12345));
    }

    #[test]
    fn test_content_length_missing() {
        let headers = Headers::new();
        assert_eq!(headers.content_length(), None);
    }

    #[test]
    fn test_content_length_invalid() {
        let mut headers = Headers::new();
        headers.add("Content-Length", "not-a-number");
        assert_eq!(headers.content_length(), None);
    }

    #[test]
    fn test_set_content_length() {
        let mut headers = Headers::new();
        headers.set_content_length(42);
        assert_eq!(headers.get("Content-Length"), Some("42"));
    }

    // ==================== Transfer-Encoding ====================

    #[test]
    fn test_is_chunked() {
        let mut headers = Headers::new();
        headers.add("Transfer-Encoding", "chunked");
        assert!(headers.is_chunked());
    }

    #[test]
    fn test_is_chunked_case_insensitive() {
        let mut headers = Headers::new();
        headers.add("Transfer-Encoding", "CHUNKED");
        assert!(headers.is_chunked());
    }

    #[test]
    fn test_is_not_chunked() {
        let mut headers = Headers::new();
        headers.add("Transfer-Encoding", "gzip");
        assert!(!headers.is_chunked());
    }

    #[test]
    fn test_is_chunked_missing() {
        let headers = Headers::new();
        assert!(!headers.is_chunked());
    }

    // ==================== Host ====================

    #[test]
    fn test_host_header() {
        let mut headers = Headers::new();
        headers.set_host("example.com");
        assert_eq!(headers.host(), Some("example.com"));
    }

    #[test]
    fn test_host_with_port() {
        let mut headers = Headers::new();
        headers.set_host("example.com:8080");
        assert_eq!(headers.host(), Some("example.com:8080"));
    }

    // ==================== Connection ====================

    #[test]
    fn test_keep_alive() {
        let mut headers = Headers::new();
        headers.add("Connection", "keep-alive");
        assert!(headers.keep_alive());
    }

    #[test]
    fn test_keep_alive_false() {
        let mut headers = Headers::new();
        headers.add("Connection", "close");
        assert!(!headers.keep_alive());
    }

    // ==================== Iteration ====================

    #[test]
    fn test_iter() {
        let mut headers = Headers::new();
        headers.add("Host", "example.com");
        headers.add("Accept", "*/*");

        let names: Vec<&str> = headers.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["Host", "Accept"]);
    }

    #[test]
    fn test_clear() {
        let mut headers = Headers::new();
        headers.add("Host", "example.com");
        headers.add("Accept", "*/*");

        headers.clear();
        assert!(headers.is_empty());
    }

    // ==================== Wire Format ====================

    #[test]
    fn test_to_wire_format() {
        let mut headers = Headers::new();
        headers.add("Host", "example.com");
        headers.add("Accept", "*/*");

        let wire = headers.to_wire_format();
        assert_eq!(wire, "Host: example.com\r\nAccept: */*\r\n");
    }

    #[test]
    fn test_to_wire_format_empty() {
        let headers = Headers::new();
        assert_eq!(headers.to_wire_format(), "");
    }

    #[test]
    fn test_from_wire_format() {
        let data = "Content-Type: text/html\r\nContent-Length: 123\r\n\r\n";
        let (headers, consumed) = Headers::from_wire_format(data);

        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("Content-Type"), Some("text/html"));
        assert_eq!(headers.content_length(), Some(123));
        assert_eq!(consumed, data.len());
    }

    #[test]
    fn test_from_wire_format_trims_whitespace() {
        let data = "Content-Type:   text/html  \r\n\r\n";
        let (headers, _) = Headers::from_wire_format(data);

        assert_eq!(headers.get("Content-Type"), Some("text/html"));
    }

    #[test]
    fn test_from_wire_format_ignores_invalid() {
        let data = "ValidHeader: value\r\nInvalidLineNoColon\r\n\r\n";
        let (headers, _) = Headers::from_wire_format(data);

        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("ValidHeader"), Some("value"));
    }

    // ==================== Real-World Headers ====================

    #[test]
    fn test_typical_request_headers() {
        let mut headers = Headers::new();
        headers.set_host("api.example.com");
        headers.set("User-Agent", "MorpheusX/1.0");
        headers.set("Accept", "*/*");
        headers.set("Connection", "close");

        assert_eq!(headers.len(), 4);
        assert!(headers
            .to_wire_format()
            .contains("Host: api.example.com\r\n"));
    }

    #[test]
    fn test_typical_response_headers() {
        let data = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Length: 1048576\r\n",
            "Connection: close\r\n",
            "\r\n"
        );

        // Skip the status line
        let headers_start = data.find("\r\n").unwrap() + 2;
        let (headers, _) = Headers::from_wire_format(&data[headers_start..]);

        assert_eq!(headers.content_type(), Some("application/octet-stream"));
        assert_eq!(headers.content_length(), Some(1048576));
        assert_eq!(headers.connection(), Some("close"));
    }

    // ==================== Header Struct ====================

    #[test]
    fn test_header_name_matches() {
        let header = Header::new("Content-Type", "text/html");
        assert!(header.name_matches("Content-Type"));
        assert!(header.name_matches("content-type"));
        assert!(header.name_matches("CONTENT-TYPE"));
        assert!(!header.name_matches("Content-Length"));
    }
}
