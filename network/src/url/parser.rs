//! URL parser for HTTP/HTTPS URLs.
//!
//! Parses URLs in the format: `scheme://host[:port][/path][?query]`
//!
//! # Examples
//!
//! ```ignore
//! use morpheus_network::url::Url;
//!
//! let url = Url::parse("http://example.com/path?query=value").unwrap();
//! assert_eq!(url.host, "example.com");
//! assert_eq!(url.path, "/path");
//! ```

use alloc::string::{String, ToString};
use crate::error::{NetworkError, Result};

/// HTTP URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    /// Parse scheme from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(Scheme::Http),
            "https" => Some(Scheme::Https),
            _ => None,
        }
    }

    /// Default port for this scheme.
    pub fn default_port(&self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }

    /// Scheme as string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// Parsed URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// URL scheme (http or https).
    pub scheme: Scheme,
    /// Host name or IP address.
    pub host: String,
    /// Optional port number.
    pub port: Option<u16>,
    /// Path component (default "/").
    pub path: String,
    /// Optional query string (without leading '?').
    pub query: Option<String>,
}

impl Url {
    /// Parse a URL string.
    ///
    /// Supports formats:
    /// - `http://host`
    /// - `http://host:port`
    /// - `http://host/path`
    /// - `http://host:port/path`
    /// - `http://host/path?query`
    /// - `https://...` (same patterns)
    ///
    /// # Errors
    ///
    /// Returns `NetworkError::InvalidUrl` if:
    /// - Missing or invalid scheme
    /// - Missing host
    /// - Invalid port number
    pub fn parse(url: &str) -> Result<Self> {
        // Find scheme separator "://"
        let scheme_end = url.find("://").ok_or(NetworkError::InvalidUrl)?;
        let scheme_str = &url[..scheme_end];
        let scheme = Scheme::parse(scheme_str).ok_or(NetworkError::InvalidUrl)?;

        // Everything after "://"
        let rest = &url[scheme_end + 3..];
        if rest.is_empty() {
            return Err(NetworkError::InvalidUrl);
        }

        // Split off query string first (if any)
        let (path_part, query) = match rest.find('?') {
            Some(idx) => {
                let q = &rest[idx + 1..];
                let query = if q.is_empty() { None } else { Some(q.to_string()) };
                (&rest[..idx], query)
            }
            None => (rest, None),
        };

        // Split host[:port] from path
        let (authority, path) = match path_part.find('/') {
            Some(idx) => (&path_part[..idx], &path_part[idx..]),
            None => (path_part, "/"),
        };

        // Parse host and optional port
        let (host, port) = Self::parse_authority(authority)?;

        if host.is_empty() {
            return Err(NetworkError::InvalidUrl);
        }

        Ok(Url {
            scheme,
            host: host.to_string(),
            port,
            path: path.to_string(),
            query,
        })
    }

    /// Parse authority (host[:port]).
    fn parse_authority(authority: &str) -> Result<(&str, Option<u16>)> {
        // Check for IPv6 address [::1]:port
        if authority.starts_with('[') {
            // IPv6 address
            let bracket_end = authority.find(']').ok_or(NetworkError::InvalidUrl)?;
            let host = &authority[..=bracket_end];
            let rest = &authority[bracket_end + 1..];

            if rest.is_empty() {
                return Ok((host, None));
            }

            if rest.starts_with(':') {
                let port_str = &rest[1..];
                let port = port_str.parse::<u16>().map_err(|_| NetworkError::InvalidUrl)?;
                return Ok((host, Some(port)));
            }

            return Err(NetworkError::InvalidUrl);
        }

        // Regular host:port
        match authority.rfind(':') {
            Some(idx) => {
                let host = &authority[..idx];
                let port_str = &authority[idx + 1..];
                
                // Validate it's actually a port (not part of IPv6)
                if port_str.is_empty() {
                    return Err(NetworkError::InvalidUrl);
                }
                
                let port = port_str.parse::<u16>().map_err(|_| NetworkError::InvalidUrl)?;
                Ok((host, Some(port)))
            }
            None => Ok((authority, None)),
        }
    }

    /// Get port, using scheme default if not specified.
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or_else(|| self.scheme.default_port())
    }

    /// Check if HTTPS.
    pub fn is_https(&self) -> bool {
        self.scheme == Scheme::Https
    }

    /// Get the full host:port string for HTTP Host header.
    pub fn host_header(&self) -> String {
        match self.port {
            Some(port) if port != self.scheme.default_port() => {
                alloc::format!("{}:{}", self.host, port)
            }
            _ => self.host.clone(),
        }
    }

    /// Get the request URI (path + query) for HTTP request line.
    pub fn request_uri(&self) -> String {
        match &self.query {
            Some(q) => alloc::format!("{}?{}", self.path, q),
            None => self.path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Scheme Tests ====================

    #[test]
    fn test_scheme_parse_http() {
        assert_eq!(Scheme::parse("http"), Some(Scheme::Http));
    }

    #[test]
    fn test_scheme_parse_https() {
        assert_eq!(Scheme::parse("https"), Some(Scheme::Https));
    }

    #[test]
    fn test_scheme_parse_invalid() {
        assert_eq!(Scheme::parse("ftp"), None);
        assert_eq!(Scheme::parse("HTTP"), None); // Case sensitive
        assert_eq!(Scheme::parse(""), None);
    }

    #[test]
    fn test_scheme_default_port() {
        assert_eq!(Scheme::Http.default_port(), 80);
        assert_eq!(Scheme::Https.default_port(), 443);
    }

    // ==================== Basic URL Parsing ====================

    #[test]
    fn test_parse_simple_http() {
        let url = Url::parse("http://example.com").unwrap();
        assert_eq!(url.scheme, Scheme::Http);
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, None);
        assert_eq!(url.path, "/");
        assert_eq!(url.query, None);
    }

    #[test]
    fn test_parse_simple_https() {
        let url = Url::parse("https://secure.example.com").unwrap();
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "secure.example.com");
        assert!(url.is_https());
    }

    #[test]
    fn test_parse_with_port() {
        let url = Url::parse("http://localhost:8080").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, Some(8080));
        assert_eq!(url.port_or_default(), 8080);
    }

    #[test]
    fn test_parse_with_path() {
        let url = Url::parse("http://example.com/path/to/file").unwrap();
        assert_eq!(url.path, "/path/to/file");
    }

    #[test]
    fn test_parse_with_query() {
        let url = Url::parse("http://example.com/search?q=test&page=1").unwrap();
        assert_eq!(url.path, "/search");
        assert_eq!(url.query, Some("q=test&page=1".to_string()));
    }

    #[test]
    fn test_parse_full_url() {
        let url = Url::parse("https://api.example.com:8443/v1/data?format=json").unwrap();
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "api.example.com");
        assert_eq!(url.port, Some(8443));
        assert_eq!(url.path, "/v1/data");
        assert_eq!(url.query, Some("format=json".to_string()));
    }

    // ==================== Port Defaults ====================

    #[test]
    fn test_port_or_default_http() {
        let url = Url::parse("http://example.com").unwrap();
        assert_eq!(url.port_or_default(), 80);
    }

    #[test]
    fn test_port_or_default_https() {
        let url = Url::parse("https://example.com").unwrap();
        assert_eq!(url.port_or_default(), 443);
    }

    #[test]
    fn test_port_or_default_explicit() {
        let url = Url::parse("http://example.com:9000").unwrap();
        assert_eq!(url.port_or_default(), 9000);
    }

    // ==================== Host Header ====================

    #[test]
    fn test_host_header_no_port() {
        let url = Url::parse("http://example.com/path").unwrap();
        assert_eq!(url.host_header(), "example.com");
    }

    #[test]
    fn test_host_header_default_port() {
        let url = Url::parse("http://example.com:80/path").unwrap();
        assert_eq!(url.host_header(), "example.com"); // Don't include default port
    }

    #[test]
    fn test_host_header_non_default_port() {
        let url = Url::parse("http://example.com:8080/path").unwrap();
        assert_eq!(url.host_header(), "example.com:8080");
    }

    // ==================== Request URI ====================

    #[test]
    fn test_request_uri_path_only() {
        let url = Url::parse("http://example.com/api/users").unwrap();
        assert_eq!(url.request_uri(), "/api/users");
    }

    #[test]
    fn test_request_uri_with_query() {
        let url = Url::parse("http://example.com/search?q=rust").unwrap();
        assert_eq!(url.request_uri(), "/search?q=rust");
    }

    // ==================== IPv4 Address ====================

    #[test]
    fn test_parse_ipv4_address() {
        let url = Url::parse("http://192.168.1.1/api").unwrap();
        assert_eq!(url.host, "192.168.1.1");
        assert_eq!(url.path, "/api");
    }

    #[test]
    fn test_parse_ipv4_with_port() {
        let url = Url::parse("http://10.0.0.1:3000").unwrap();
        assert_eq!(url.host, "10.0.0.1");
        assert_eq!(url.port, Some(3000));
    }

    // ==================== IPv6 Address ====================

    #[test]
    fn test_parse_ipv6_localhost() {
        let url = Url::parse("http://[::1]/test").unwrap();
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.path, "/test");
    }

    #[test]
    fn test_parse_ipv6_with_port() {
        let url = Url::parse("http://[::1]:8080/api").unwrap();
        assert_eq!(url.host, "[::1]");
        assert_eq!(url.port, Some(8080));
    }

    #[test]
    fn test_parse_ipv6_full() {
        let url = Url::parse("http://[2001:db8::1]:9000/path").unwrap();
        assert_eq!(url.host, "[2001:db8::1]");
        assert_eq!(url.port, Some(9000));
    }

    // ==================== Error Cases ====================

    #[test]
    fn test_parse_missing_scheme() {
        assert!(Url::parse("example.com/path").is_err());
    }

    #[test]
    fn test_parse_invalid_scheme() {
        assert!(Url::parse("ftp://example.com").is_err());
    }

    #[test]
    fn test_parse_missing_host() {
        assert!(Url::parse("http://").is_err());
    }

    #[test]
    fn test_parse_invalid_port() {
        assert!(Url::parse("http://example.com:abc").is_err());
    }

    #[test]
    fn test_parse_port_overflow() {
        assert!(Url::parse("http://example.com:99999").is_err());
    }

    #[test]
    fn test_parse_empty_port() {
        assert!(Url::parse("http://example.com:").is_err());
    }

    #[test]
    fn test_parse_empty_string() {
        assert!(Url::parse("").is_err());
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_parse_root_path() {
        let url = Url::parse("http://example.com/").unwrap();
        assert_eq!(url.path, "/");
    }

    #[test]
    fn test_parse_empty_query() {
        let url = Url::parse("http://example.com/path?").unwrap();
        assert_eq!(url.query, None); // Empty query treated as None
    }

    #[test]
    fn test_parse_query_without_value() {
        let url = Url::parse("http://example.com/path?key").unwrap();
        assert_eq!(url.query, Some("key".to_string()));
    }

    // ==================== Real-World URLs ====================

    #[test]
    fn test_parse_ubuntu_iso() {
        let url = Url::parse(
            "http://releases.ubuntu.com/24.04/ubuntu-24.04-live-server-amd64.iso"
        ).unwrap();
        assert_eq!(url.host, "releases.ubuntu.com");
        assert_eq!(url.path, "/24.04/ubuntu-24.04-live-server-amd64.iso");
    }

    #[test]
    fn test_parse_arch_mirror() {
        let url = Url::parse(
            "https://mirror.rackspace.com/archlinux/iso/latest/archlinux-x86_64.iso"
        ).unwrap();
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "mirror.rackspace.com");
    }

    #[test]
    fn test_parse_localhost_dev() {
        let url = Url::parse("http://localhost:3000/api/v1/users?limit=10").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, Some(3000));
        assert_eq!(url.path, "/api/v1/users");
        assert_eq!(url.query, Some("limit=10".to_string()));
    }
}
