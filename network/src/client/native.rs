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

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::Ipv4Addr;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State as TcpState;

use crate::client::HttpClient;
use crate::device::NetworkDevice;
use crate::error::{NetworkError, Result};
use crate::http::{Headers, Request, Response};
use crate::stack::{NetConfig, NetInterface, NetState};
use crate::types::{HttpMethod, ProgressCallback};
use crate::url::Url;

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
    pub fn wait_for_network(&mut self, timeout_ms: u64) -> Result<()> {
        let start = self.now();

        while !self.iface.has_ip() {
            self.poll();

            if self.now() - start > timeout_ms {
                return Err(NetworkError::Timeout);
            }

            crate::device::pci::tsc_delay_us(1000); // 1ms
        }

        Ok(())
    }

    // ========================================================================
    // DNS Resolution
    // ========================================================================

    /// Resolve hostname to IP address using DNS or hardcoded fallbacks.
    pub fn resolve_host(&mut self, host: &str) -> Result<Ipv4Addr> {
        crate::stack::debug_log(40, "resolve_host start");

        // Try parsing as IP address first
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            crate::stack::debug_log(41, "host is IP addr");
            return Ok(ip);
        }

        // Try actual DNS resolution
        if let Ok(ip) = self.try_dns_query(host) {
            return Ok(ip);
        }

        // Fallback to hardcoded DNS
        self.lookup_hardcoded_dns(host)
    }

    /// Attempt DNS resolution via UDP query.
    fn try_dns_query(&mut self, host: &str) -> Result<Ipv4Addr> {
        crate::stack::debug_log(42, "starting DNS query");

        let start = self.now();
        let timeout_ms = 5000;

        let query_handle = self.iface.start_dns_query(host).map_err(|_| {
            crate::stack::debug_log(47, "DNS start failed");
            NetworkError::DnsResolutionFailed
        })?;

        crate::stack::debug_log(43, "DNS query started");

        // Poll until result or timeout
        loop {
            let now = self.now();
            self.iface.poll(now);

            match self.iface.get_dns_result(query_handle) {
                Ok(Some(ip)) => {
                    crate::stack::debug_log(44, "DNS resolved OK");
                    return Ok(ip);
                }
                Ok(None) => {
                    if now - start > timeout_ms {
                        crate::stack::debug_log(45, "DNS timeout");
                        return Err(NetworkError::DnsResolutionFailed);
                    }
                    crate::device::pci::tsc_delay_us(1000); // 1ms
                }
                Err(_) => {
                    crate::stack::debug_log(46, "DNS query failed");
                    return Err(NetworkError::DnsResolutionFailed);
                }
            }
        }
    }

    /// Lookup host in hardcoded DNS table.
    fn lookup_hardcoded_dns(&self, host: &str) -> Result<Ipv4Addr> {
        const KNOWN_HOSTS: &[(&str, &str)] = &[
            ("speedtest.tele2.net", "90.130.70.73"),
            ("mirror.fcix.net", "204.152.191.37"),
            ("ftp.acc.umu.se", "130.239.18.159"),
            ("releases.ubuntu.com", "91.189.91.38"),
            ("cdimage.ubuntu.com", "91.189.88.142"),
        ];

        for (hostname, ip_str) in KNOWN_HOSTS {
            if host == *hostname {
                if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                    crate::stack::debug_log(48, "using hardcoded DNS");
                    return Ok(ip);
                }
            }
        }

        crate::stack::debug_log(49, "DNS resolution FAILED");
        Err(NetworkError::DnsResolutionFailed)
    }

    // ========================================================================
    // TCP Connection Management
    // ========================================================================

    /// Connect to a remote host.
    fn connect(&mut self, ip: Ipv4Addr, port: u16) -> Result<()> {
        crate::stack::debug_log(50, "TCP connect start");

        self.close_existing_socket();
        let handle = self.create_tcp_socket()?;
        self.initiate_connection(handle, ip, port)?;
        self.wait_for_connection(handle)?;

        crate::stack::debug_log(55, "TCP connected OK");
        Ok(())
    }

    /// Close any existing socket.
    fn close_existing_socket(&mut self) {
        if let Some(handle) = self.socket.take() {
            self.iface.tcp_close(handle);
            self.iface.remove_socket(handle);
        }
    }

    /// Create a new TCP socket.
    fn create_tcp_socket(&mut self) -> Result<SocketHandle> {
        let handle = self.iface.tcp_socket()?;
        self.socket = Some(handle);
        crate::stack::debug_log(51, "TCP socket created");
        Ok(handle)
    }

    /// Initiate TCP connection.
    fn initiate_connection(&mut self, handle: SocketHandle, ip: Ipv4Addr, port: u16) -> Result<()> {
        self.iface.tcp_connect(handle, ip, port)?;
        crate::stack::debug_log(52, "TCP connecting...");
        Ok(())
    }

    /// Wait for TCP connection to complete.
    fn wait_for_connection(&mut self, handle: SocketHandle) -> Result<()> {
        let start = self.now();

        while !self.iface.tcp_is_connected(handle) {
            self.poll();

            let state = self.iface.tcp_state(handle);
            if state == TcpState::Closed || state == TcpState::TimeWait {
                crate::stack::debug_log(53, "TCP conn FAILED");
                return Err(NetworkError::ConnectionFailed);
            }

            if self.now() - start > self.config.connect_timeout_ms {
                crate::stack::debug_log(54, "TCP conn TIMEOUT");
                return Err(NetworkError::Timeout);
            }

            crate::device::pci::tsc_delay_us(1000); // 1ms
        }

        Ok(())
    }

    // ========================================================================
    // Data Transfer
    // ========================================================================

    /// Send all data and wait for transmission to complete.
    fn send_all(&mut self, data: &[u8]) -> Result<()> {
        let handle = self.socket.ok_or(NetworkError::NotConnected)?;
        let start = self.now();
        let mut sent = 0;

        while sent < data.len() {
            self.poll();

            if self.iface.tcp_can_send(handle) {
                let n = self.iface.tcp_send(handle, &data[sent..])?;
                sent += n;
            }

            if self.now() - start > self.config.read_timeout_ms {
                return Err(NetworkError::Timeout);
            }

            crate::device::pci::tsc_delay_us(100); // 100us
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
                return Ok(0);
            }

            if self.now() - start > self.config.read_timeout_ms {
                return Err(NetworkError::Timeout);
            }

            crate::device::pci::tsc_delay_us(1000); // 1ms
        }
    }

    // ========================================================================
    // HTTP Response Reading
    // ========================================================================

    /// Read HTTP headers until \r\n\r\n found.
    fn read_headers(&mut self) -> Result<Vec<u8>> {
        let mut header_buf = Vec::new();
        let mut buffer = [0u8; 4096];

        loop {
            let n = self.recv(&mut buffer)?;
            if n == 0 {
                crate::stack::debug_log(68, "unexpected EOF");
                return Err(NetworkError::UnexpectedEof);
            }

            header_buf.extend_from_slice(&buffer[..n]);

            if find_header_end(&header_buf).is_some() {
                return Ok(header_buf);
            }
        }
    }

    /// Read full HTTP response (headers + body).
    fn read_full_response(&mut self) -> Result<Vec<u8>> {
        let mut response_data = self.read_headers()?;
        let body_start = find_header_end(&response_data).ok_or(NetworkError::InvalidResponse)? + 4;

        let headers_str = core::str::from_utf8(&response_data[..body_start - 4])
            .map_err(|_| NetworkError::InvalidResponse)?;
        let content_length = parse_content_length(headers_str);

        // Read remaining body
        self.read_remaining_body(&mut response_data, body_start, content_length)?;

        Ok(response_data)
    }

    /// Read remaining body data based on Content-Length.
    fn read_remaining_body(
        &mut self,
        response_data: &mut Vec<u8>,
        body_start: usize,
        content_length: Option<usize>,
    ) -> Result<()> {
        let mut buffer = [0u8; 4096];
        let mut total_body = response_data.len() - body_start;

        loop {
            if let Some(expected) = content_length {
                if total_body >= expected {
                    break;
                }
            }

            match self.recv(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    response_data.extend_from_slice(&buffer[..n]);
                    total_body += n;

                    if response_data.len() > self.config.max_response_size {
                        return Err(NetworkError::ResponseTooLarge);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// Stream response body to callback.
    fn stream_response_body<F>(
        &mut self,
        initial_body: &[u8],
        content_length: Option<usize>,
        callback: &mut F,
    ) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        // Send initial body data that was already received
        let mut total = initial_body.len();
        if !initial_body.is_empty() {
            callback(initial_body)?;
        }

        // Stream remaining body
        let mut buffer = [0u8; 4096];
        loop {
            if let Some(expected) = content_length {
                if total >= expected {
                    break;
                }
            }

            match self.recv(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    callback(&buffer[..n])?;
                    total += n;
                }
                Err(e) => {
                    crate::stack::debug_log(71, "recv error");
                    return Err(e);
                }
            }
        }

        crate::stack::debug_log(72, "download complete");
        Ok(total)
    }

    // ========================================================================
    // HTTP Request Execution
    // ========================================================================

    /// Execute a basic HTTP request and return full response.
    fn do_request(&mut self, request: &Request) -> Result<Response> {
        let ip = self.resolve_host(&request.url.host)?;
        let port = request.url.port.unwrap_or(80);

        self.connect(ip, port)?;
        self.send_all(&request.to_wire_format())?;

        let response_data = self.read_full_response()?;
        let (response, _) = Response::parse(&response_data)?;

        Ok(response)
    }

    /// Execute request with automatic redirect following.
    fn do_request_with_redirects(&mut self, mut request: Request) -> Result<Response> {
        let mut redirects = 0;

        loop {
            let response = self.do_request(&request)?;

            if !response.is_redirect() || !self.config.follow_redirects {
                return Ok(response);
            }

            if redirects >= self.config.max_redirects {
                return Err(NetworkError::TooManyRedirects);
            }

            let location = response.location().ok_or(NetworkError::InvalidResponse)?;

            request = self.build_redirect_request(&request, location)?;
            redirects += 1;
        }
    }

    /// Build a new request for a redirect location.
    fn build_redirect_request(&self, original: &Request, location: &str) -> Result<Request> {
        let new_url = if location.starts_with("http://") || location.starts_with("https://") {
            Url::parse(location)?
        } else {
            let mut new = original.url.clone();
            new.path = location.to_string();
            new
        };

        Ok(Request::get(new_url))
    }

    // ========================================================================
    // Public HTTP Methods
    // ========================================================================

    /// Simple GET request returning full response.
    pub fn get(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::get(parsed_url);
        self.do_request_with_redirects(request)
    }

    /// GET request with streaming callback for large downloads.
    pub fn get_streaming<F>(&mut self, url: &str, mut callback: F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        crate::stack::debug_log(60, "get_streaming start");

        let parsed_url = Url::parse(url)?;

        if parsed_url.is_https() {
            crate::stack::debug_log(61, "HTTPS not supported!");
            return Err(NetworkError::TlsNotSupported);
        }

        self.execute_streaming_request(&parsed_url, &mut callback)
    }

    /// Execute a streaming HTTP request.
    fn execute_streaming_request<F>(&mut self, url: &Url, callback: &mut F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        crate::stack::debug_log(62, "resolving host...");
        let ip = self.resolve_host(&url.host)?;
        crate::stack::debug_log(63, "host resolved");

        let port = url.port.unwrap_or_else(|| url.scheme.default_port());
        self.connect(ip, port)?;
        crate::stack::debug_log(64, "connected to server");

        crate::stack::debug_log(65, "sending HTTP request");
        let request = Request::get(url.clone());
        self.send_all(&request.to_wire_format())?;
        crate::stack::debug_log(66, "request sent");

        self.stream_response(callback)
    }

    /// Stream the HTTP response to the callback.
    fn stream_response<F>(&mut self, callback: &mut F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        crate::stack::debug_log(67, "reading headers...");
        let header_buf = self.read_headers()?;

        let body_start = find_header_end(&header_buf).ok_or(NetworkError::InvalidResponse)? + 4;

        crate::stack::debug_log(69, "headers received");

        let headers_str = core::str::from_utf8(&header_buf[..body_start - 4])
            .map_err(|_| NetworkError::InvalidResponse)?;
        let content_length = parse_content_length(headers_str);

        let initial_body = &header_buf[body_start..];

        crate::stack::debug_log(70, "streaming body...");
        self.stream_response_body(initial_body, content_length, callback)
    }

    // ========================================================================
    // Connection Management
    // ========================================================================

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

// ============================================================================
// Helper Functions
// ============================================================================

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
        // find_header_end returns position of \r\n\r\n start, not end
        // "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello"
        //  0                16                 34   38
        let data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello";
        assert_eq!(find_header_end(data), Some(34));

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
