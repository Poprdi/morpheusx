//! Native HTTP client using bare metal TCP/IP stack.
//!
//! This client uses smoltcp over a `NetworkDevice` driver directly,
//! bypassing UEFI protocols entirely. Works with any network hardware
//! that implements the `NetworkDevice` trait.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                   NativeHttpClient<D>                       │
//! │  (HTTP/1.1 request/response, redirects, streaming)          │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                   NetInterface<D>                           │
//! │  (TCP sockets, DHCP, IP routing via smoltcp)               │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │              NetworkDevice (VirtIO, Intel, etc.)            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use morpheus_network::client::native::NativeHttpClient;
//! use morpheus_network::device::virtio::VirtioNetDevice;
//! use morpheus_network::stack::NetConfig;
//!
//! // Create device
//! let device = VirtioNetDevice::new(transport)?;
//!
//! // Create HTTP client (handles DHCP internally)
//! let mut client = NativeHttpClient::new(device, NetConfig::dhcp())?;
//!
//! // Wait for network ready
//! client.wait_for_network(30_000)?; // 30 second timeout
//!
//! // Make HTTP request
//! let response = client.get("http://example.com/file.iso")?;
//! ```

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use core::net::Ipv4Addr;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State as TcpState;

use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};
use crate::http::{Request, Response, Headers};
use crate::stack::{NetInterface, NetConfig, NetState};
use crate::url::Url;
use crate::types::{HttpMethod, ProgressCallback};
use crate::client::HttpClient;

/// Default timeout for operations (milliseconds).
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Maximum response size (10GB for ISOs).
pub const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Maximum redirects to follow.
pub const MAX_REDIRECTS: u32 = 10;

/// Native HTTP client configuration.
#[derive(Debug, Clone)]
pub struct NativeClientConfig {
    /// Timeout for connect operations (ms).
    pub connect_timeout_ms: u64,
    /// Timeout for read operations (ms).
    pub read_timeout_ms: u64,
    /// Maximum response body size.
    pub max_response_size: usize,
    /// Follow redirects automatically.
    pub follow_redirects: bool,
    /// Maximum redirects to follow.
    pub max_redirects: u32,
    /// Buffer size for streaming.
    pub buffer_size: usize,
}

impl Default for NativeClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 30_000,
            read_timeout_ms: 60_000,
            max_response_size: MAX_RESPONSE_SIZE,
            follow_redirects: true,
            max_redirects: MAX_REDIRECTS,
            buffer_size: 64 * 1024, // 64KB
        }
    }
}

impl NativeClientConfig {
    /// Config for downloading large files.
    pub fn for_large_downloads() -> Self {
        Self {
            connect_timeout_ms: 60_000,
            read_timeout_ms: 120_000,
            max_response_size: MAX_RESPONSE_SIZE,
            follow_redirects: true,
            max_redirects: MAX_REDIRECTS,
            buffer_size: 256 * 1024, // 256KB
        }
    }
}

/// Native bare-metal HTTP client.
///
/// Generic over `NetworkDevice` so it works with any driver:
/// - VirtIO (QEMU, KVM)
/// - Intel NICs (future)
/// - Realtek NICs (future)
/// - etc.
pub struct NativeHttpClient<D: NetworkDevice> {
    /// Network interface with TCP/IP stack.
    iface: NetInterface<D>,
    /// Client configuration.
    config: NativeClientConfig,
    /// Current TCP socket handle (if connected).
    socket: Option<SocketHandle>,
    /// Function to get current time in milliseconds.
    /// Must be provided by the caller (platform-specific).
    get_time_ms: fn() -> u64,
}

impl<D: NetworkDevice> NativeHttpClient<D> {
    /// Create a new native HTTP client.
    ///
    /// # Arguments
    ///
    /// * `device` - Network device to use
    /// * `net_config` - IP configuration (DHCP or static)
    /// * `get_time_ms` - Function returning current time in milliseconds
    pub fn new(device: D, net_config: NetConfig, get_time_ms: fn() -> u64) -> Self {
        let iface = NetInterface::new(device, net_config);
        Self {
            iface,
            config: NativeClientConfig::default(),
            socket: None,
            get_time_ms,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(
        device: D,
        net_config: NetConfig,
        client_config: NativeClientConfig,
        get_time_ms: fn() -> u64,
    ) -> Self {
        let iface = NetInterface::new(device, net_config);
        Self {
            iface,
            config: client_config,
            socket: None,
            get_time_ms,
        }
    }

    /// Get current timestamp.
    fn now(&self) -> u64 {
        (self.get_time_ms)()
    }

    /// Poll the network interface.
    pub fn poll(&mut self) {
        self.iface.poll(self.now());
    }

    /// Check if network is ready (has IP address).
    pub fn is_network_ready(&self) -> bool {
        self.iface.has_ip()
    }

    /// Get current IP address.
    pub fn ip_address(&self) -> Option<Ipv4Addr> {
        self.iface.ipv4_addr()
    }

    /// Wait for network to be ready (DHCP complete or static configured).
    ///
    /// Returns error if timeout expires.
    pub fn wait_for_network(&mut self, timeout_ms: u64) -> Result<()> {
        let start = self.now();
        
        while !self.iface.has_ip() {
            self.poll();
            
            if self.now() - start > timeout_ms {
                return Err(NetworkError::Timeout);
            }
            
            // Small delay to avoid busy-spinning
            // In real implementation, could use timer or yield
        }
        
        Ok(())
    }

    /// Resolve hostname to IP address.
    ///
    /// Supports:
    /// 1. Numeric IP addresses (e.g., "192.168.1.1")
    /// 2. Hardcoded DNS for common distro download hosts
    /// 3. TODO: Full DNS resolution via UDP
    pub fn resolve_host(&self, host: &str) -> Result<Ipv4Addr> {
        // Try parsing as IP address first
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return Ok(ip);
        }

        // Hardcoded DNS for common distro download hosts
        // These are stable IPs for CDN/mirror services
        // Updated: January 2026
        let known_hosts: &[(&str, &str)] = &[
            // Test endpoints (HTTP)
            ("speedtest.tele2.net", "90.130.70.73"),
            // Tails HTTP mirrors
            ("mirror.fcix.net", "204.152.191.37"),
            ("ftp.acc.umu.se", "130.239.18.159"),
            // Ubuntu/Canonical
            ("releases.ubuntu.com", "91.189.91.38"),
            ("cdimage.ubuntu.com", "91.189.88.142"),
            ("archive.ubuntu.com", "91.189.91.39"),
            // Kernel.org mirrors
            ("mirrors.edge.kernel.org", "147.75.197.195"),
            ("cdn.kernel.org", "139.178.84.217"),
            // Tails official (HTTPS only)
            ("download.tails.net", "204.13.164.63"),
            // Fedora
            ("download.fedoraproject.org", "152.19.134.142"),
            // Debian  
            ("cdimage.debian.org", "194.71.11.165"),
            ("ftp.debian.org", "199.232.66.132"),
            // Arch Linux
            ("mirror.rackspace.com", "162.209.85.197"),
            ("mirrors.kernel.org", "147.75.197.195"),
            // Kali
            ("cdimage.kali.org", "192.99.200.113"),
            // Whonix
            ("download.whonix.org", "116.202.120.184"),
            // Generic/test
            ("httpbin.org", "54.144.42.194"),
        ];

        for (hostname, ip_str) in known_hosts {
            if host == *hostname || host.ends_with(&format!(".{}", hostname)) {
                if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                    return Ok(ip);
                }
            }
        }

        // TODO: Implement DNS resolution via UDP
        // For now, return error for unknown hostnames
        Err(NetworkError::DnsResolutionFailed)
    }

    /// Connect to a remote host.
    fn connect(&mut self, ip: Ipv4Addr, port: u16) -> Result<()> {
        // Close any existing connection
        if let Some(handle) = self.socket.take() {
            self.iface.tcp_close(handle);
            self.iface.remove_socket(handle);
        }

        // Create new socket
        let handle = self.iface.tcp_socket()?;
        self.socket = Some(handle);

        // Initiate connection
        self.iface.tcp_connect(handle, ip, port)?;

        // Wait for connection
        let start = self.now();
        while !self.iface.tcp_is_connected(handle) {
            self.poll();

            let state = self.iface.tcp_state(handle);
            if state == TcpState::Closed || state == TcpState::TimeWait {
                return Err(NetworkError::ConnectionFailed);
            }

            if self.now() - start > self.config.connect_timeout_ms {
                return Err(NetworkError::Timeout);
            }
        }

        Ok(())
    }

    /// Send data and wait for it to be transmitted.
    fn send_all(&mut self, data: &[u8]) -> Result<()> {
        let handle = self.socket.ok_or(NetworkError::NotConnected)?;
        let mut sent = 0;
        let start = self.now();

        while sent < data.len() {
            self.poll();

            if self.iface.tcp_can_send(handle) {
                let n = self.iface.tcp_send(handle, &data[sent..])?;
                sent += n;
            }

            if self.now() - start > self.config.read_timeout_ms {
                return Err(NetworkError::Timeout);
            }
        }

        Ok(())
    }

    /// Receive data with timeout.
    fn recv(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let handle = self.socket.ok_or(NetworkError::NotConnected)?;
        let start = self.now();

        loop {
            self.poll();

            if self.iface.tcp_can_recv(handle) {
                return self.iface.tcp_recv(handle, buffer);
            }

            let state = self.iface.tcp_state(handle);
            if state == TcpState::Closed || state == TcpState::CloseWait {
                return Ok(0); // Connection closed
            }

            if self.now() - start > self.config.read_timeout_ms {
                return Err(NetworkError::Timeout);
            }
        }
    }

    /// Execute an HTTP request.
    fn do_request(&mut self, request: &Request) -> Result<Response> {
        let url = &request.url;
        
        // Resolve host
        let ip = self.resolve_host(&url.host)?;
        let port = url.port.unwrap_or(80);

        // Connect
        self.connect(ip, port)?;

        // Build and send request
        let wire = request.to_wire_format();
        self.send_all(&wire)?;

        // Read response
        let mut response_data = Vec::new();
        let mut buffer = [0u8; 4096];

        loop {
            match self.recv(&mut buffer) {
                Ok(0) => break, // Connection closed
                Ok(n) => {
                    response_data.extend_from_slice(&buffer[..n]);
                    
                    // Check for response size limit
                    if response_data.len() > self.config.max_response_size {
                        return Err(NetworkError::ResponseTooLarge);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Parse response
        let (response, _consumed) = Response::parse(&response_data)?;
        Ok(response)
    }

    /// Execute request with redirect handling.
    fn do_request_with_redirects(&mut self, mut request: Request) -> Result<Response> {
        let mut redirects = 0;

        loop {
            let response = self.do_request(&request)?;

            if response.is_redirect() && self.config.follow_redirects {
                if redirects >= self.config.max_redirects {
                    return Err(NetworkError::TooManyRedirects);
                }

                if let Some(location) = response.location() {
                    // Parse redirect URL (may be relative)
                    let new_url = if location.starts_with("http://") || location.starts_with("https://") {
                        Url::parse(location)?
                    } else {
                        // Relative URL - combine with original
                        let mut new = request.url.clone();
                        new.path = location.to_string();
                        new
                    };

                    request = Request::get(new_url);
                    redirects += 1;
                    continue;
                }
            }

            return Ok(response);
        }
    }

    /// Simple GET request.
    pub fn get(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::get(parsed_url);
        self.do_request_with_redirects(request)
    }

    /// GET request with streaming callback for large downloads.
    ///
    /// Calls the callback with chunks of data as they arrive.
    pub fn get_streaming<F>(&mut self, url: &str, mut callback: F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        let parsed_url = Url::parse(url)?;
        
        // Check for HTTPS - we don't support TLS yet!
        if parsed_url.is_https() {
            return Err(NetworkError::TlsNotSupported);
        }
        
        let request = Request::get(parsed_url.clone());

        // Resolve and connect
        let ip = self.resolve_host(&request.url.host)?;
        let port = request.url.port.unwrap_or_else(|| parsed_url.scheme.default_port());
        self.connect(ip, port)?;

        // Send request
        let wire = request.to_wire_format();
        self.send_all(&wire)?;

        // Read response headers first
        let mut header_buf = Vec::new();
        let mut buffer = [0u8; 4096];
        let mut headers_complete = false;
        let mut body_start = 0;

        while !headers_complete {
            let n = self.recv(&mut buffer)?;
            if n == 0 {
                return Err(NetworkError::UnexpectedEof);
            }
            header_buf.extend_from_slice(&buffer[..n]);

            // Look for header/body separator
            if let Some(pos) = find_header_end(&header_buf) {
                headers_complete = true;
                body_start = pos + 4; // Skip \r\n\r\n
            }
        }

        // Parse headers to get Content-Length
        let headers_str = core::str::from_utf8(&header_buf[..body_start - 4])
            .map_err(|_| NetworkError::InvalidResponse)?;
        
        let content_length = parse_content_length(headers_str);
        
        // Handle any body data already received with headers
        let initial_body = &header_buf[body_start..];
        let mut total_received = initial_body.len();
        if !initial_body.is_empty() {
            callback(initial_body)?;
        }

        // Stream remaining body
        loop {
            // Check if we have all data
            if let Some(expected) = content_length {
                if total_received >= expected {
                    break;
                }
            }

            match self.recv(&mut buffer) {
                Ok(0) => break, // Connection closed
                Ok(n) => {
                    callback(&buffer[..n])?;
                    total_received += n;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(total_received)
    }

    /// Close any active connection.
    pub fn close(&mut self) {
        if let Some(handle) = self.socket.take() {
            self.iface.tcp_close(handle);
            // Poll to send FIN
            for _ in 0..10 {
                self.poll();
            }
            self.iface.remove_socket(handle);
        }
    }

    /// Get reference to the network interface.
    pub fn interface(&self) -> &NetInterface<D> {
        &self.iface
    }

    /// Get mutable reference to the network interface.
    pub fn interface_mut(&mut self) -> &mut NetInterface<D> {
        &mut self.iface
    }
}

impl<D: NetworkDevice> Drop for NativeHttpClient<D> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<D: NetworkDevice> HttpClient for NativeHttpClient<D> {
    fn request(&mut self, request: &Request) -> Result<Response> {
        self.do_request_with_redirects(request.clone())
    }

    fn request_with_progress(
        &mut self,
        request: &Request,
        _progress: ProgressCallback,
    ) -> Result<Response> {
        // TODO: Implement progress tracking
        self.request(request)
    }

    fn is_ready(&self) -> bool {
        self.iface.has_ip()
    }
}

/// Find the end of HTTP headers (\r\n\r\n).
fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

/// Parse Content-Length from headers string.
fn parse_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-length:") {
            let value = line.split(':').nth(1)?.trim();
            return value.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_header_end() {
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello";
        assert_eq!(find_header_end(data), Some(36));

        let data = b"No headers here";
        assert_eq!(find_header_end(data), None);
    }

    #[test]
    fn test_parse_content_length() {
        let headers = "HTTP/1.1 200 OK\r\nContent-Length: 12345\r\nContent-Type: text/plain";
        assert_eq!(parse_content_length(headers), Some(12345));

        let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/plain";
        assert_eq!(parse_content_length(headers), None);
    }
}
