//! HTTP/1.1 client over smoltcp, generic over any `NetworkDevice` driver.

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::Ipv4Addr;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State as TcpState;

use crate::client::HttpClient;
use crate::error::{NetworkError, Result};
use crate::http::{Headers, Request, Response};
use crate::stack::{NetConfig, NetInterface, NetState};
use crate::types::{HttpMethod, ProgressCallback};
use crate::url::Url;
use morpheus_nic::device::NetworkDevice;

pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Maximum response size (10GB for ISOs).
pub const MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024 * 1024;

pub const MAX_REDIRECTS: u32 = 10;

#[derive(Debug, Clone)]
pub struct NativeClientConfig {
    pub connect_timeout_ms: u64,
    pub read_timeout_ms: u64,
    pub max_response_size: usize,
    pub follow_redirects: bool,
    pub max_redirects: u32,
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

pub struct NativeHttpClient<D: NetworkDevice> {
    iface: NetInterface<D>,
    config: NativeClientConfig,
    socket: Option<SocketHandle>,
    /// Platform clock; supplied by caller.
    get_time_ms: fn() -> u64,
}

impl<D: NetworkDevice> NativeHttpClient<D> {
    pub fn new(device: D, net_config: NetConfig, get_time_ms: fn() -> u64) -> Self {
        let iface = NetInterface::new(device, net_config);
        Self {
            iface,
            config: NativeClientConfig::default(),
            socket: None,
            get_time_ms,
        }
    }

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

    fn now(&self) -> u64 {
        (self.get_time_ms)()
    }

    pub fn poll(&mut self) {
        self.iface.poll(self.now());
    }

    pub fn is_network_ready(&self) -> bool {
        self.iface.has_ip()
    }

    pub fn ip_address(&self) -> Option<Ipv4Addr> {
        self.iface.ipv4_addr()
    }

    /// Block until DHCP completes or static config is applied.
    pub fn wait_for_network(&mut self, timeout_ms: u64) -> Result<()> {
        let start = self.now();

        while !self.iface.has_ip() {
            self.poll();

            if self.now() - start > timeout_ms {
                return Err(NetworkError::Timeout);
            }

            morpheus_nic::device::pci::tsc_delay_us(1000); // 1ms
        }

        Ok(())
    }

    /// Resolve via literal parse, then DNS, then hardcoded fallback table.
    pub fn resolve_host(&mut self, host: &str) -> Result<Ipv4Addr> {
        crate::stack::debug_log(40, "resolve_host start");

        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            crate::stack::debug_log(41, "host is IP addr");
            return Ok(ip);
        }

        if let Ok(ip) = self.try_dns_query(host) {
            return Ok(ip);
        }

        self.lookup_hardcoded_dns(host)
    }

    fn try_dns_query(&mut self, host: &str) -> Result<Ipv4Addr> {
        crate::stack::debug_log(42, "starting DNS query");

        let start = self.now();
        let timeout_ms = 5000;

        let query_handle = self.iface.start_dns_query(host).map_err(|_| {
            crate::stack::debug_log(47, "DNS start failed");
            NetworkError::DnsResolutionFailed
        })?;

        crate::stack::debug_log(43, "DNS query started");

        loop {
            let now = self.now();
            self.iface.poll(now);

            match self.iface.get_dns_result(query_handle) {
                Ok(Some(ip)) => {
                    crate::stack::debug_log(44, "DNS resolved OK");
                    return Ok(ip);
                },
                Ok(None) => {
                    if now - start > timeout_ms {
                        crate::stack::debug_log(45, "DNS timeout");
                        return Err(NetworkError::DnsResolutionFailed);
                    }
                    morpheus_nic::device::pci::tsc_delay_us(1000); // 1ms
                },
                Err(_) => {
                    crate::stack::debug_log(46, "DNS query failed");
                    return Err(NetworkError::DnsResolutionFailed);
                },
            }
        }
    }

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

    fn connect(&mut self, ip: Ipv4Addr, port: u16) -> Result<()> {
        crate::stack::debug_log(50, "TCP connect start");

        self.close_existing_socket();
        let handle = self.create_tcp_socket()?;
        self.initiate_connection(handle, ip, port)?;
        self.wait_for_connection(handle)?;

        crate::stack::debug_log(55, "TCP connected OK");
        Ok(())
    }

    fn close_existing_socket(&mut self) {
        if let Some(handle) = self.socket.take() {
            self.iface.tcp_close(handle);
            self.iface.remove_socket(handle);
        }
    }

    fn create_tcp_socket(&mut self) -> Result<SocketHandle> {
        let handle = self.iface.tcp_socket()?;
        self.socket = Some(handle);
        crate::stack::debug_log(51, "TCP socket created");
        Ok(handle)
    }

    fn initiate_connection(&mut self, handle: SocketHandle, ip: Ipv4Addr, port: u16) -> Result<()> {
        self.iface.tcp_connect(handle, ip, port)?;
        crate::stack::debug_log(52, "TCP connecting...");
        Ok(())
    }

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

            morpheus_nic::device::pci::tsc_delay_us(1000); // 1ms
        }

        Ok(())
    }

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

            morpheus_nic::device::pci::tsc_delay_us(100); // 100us
        }

        Ok(())
    }

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

            morpheus_nic::device::pci::tsc_delay_us(1000); // 1ms
        }
    }

    /// Read until the \r\n\r\n header terminator.
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

    fn read_full_response(&mut self) -> Result<Vec<u8>> {
        let mut response_data = self.read_headers()?;
        let body_start = find_header_end(&response_data).ok_or(NetworkError::InvalidResponse)? + 4;

        let headers_str = core::str::from_utf8(&response_data[..body_start - 4])
            .map_err(|_| NetworkError::InvalidResponse)?;
        let content_length = parse_content_length(headers_str);

        self.read_remaining_body(&mut response_data, body_start, content_length)?;

        Ok(response_data)
    }

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
                },
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    fn stream_response_body<F>(
        &mut self,
        initial_body: &[u8],
        content_length: Option<usize>,
        callback: &mut F,
    ) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        // Flush body bytes already pulled in with the headers.
        let mut total = initial_body.len();
        if !initial_body.is_empty() {
            callback(initial_body)?;
        }

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
                },
                Err(e) => {
                    crate::stack::debug_log(71, "recv error");
                    return Err(e);
                },
            }
        }

        crate::stack::debug_log(72, "download complete");
        Ok(total)
    }

    fn do_request(&mut self, request: &Request) -> Result<Response> {
        let ip = self.resolve_host(&request.url.host)?;
        let port = request.url.port.unwrap_or(80);

        self.connect(ip, port)?;
        self.send_all(&request.to_wire_format())?;

        let response_data = self.read_full_response()?;
        let (response, _) = Response::parse(&response_data)?;

        Ok(response)
    }

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

    pub fn get(&mut self, url: &str) -> Result<Response> {
        let parsed_url = Url::parse(url)?;
        let request = Request::get(parsed_url);
        self.do_request_with_redirects(request)
    }

    /// GET that streams the body to `callback`; for large downloads.
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

    pub fn close(&mut self) {
        if let Some(handle) = self.socket.take() {
            self.iface.tcp_close(handle);

            // Poll to flush the FIN.
            for _ in 0..10 {
                self.poll();
            }

            self.iface.remove_socket(handle);
        }
    }

    pub fn interface(&self) -> &NetInterface<D> {
        &self.iface
    }

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

fn find_header_end(data: &[u8]) -> Option<usize> {
    (0..data.len().saturating_sub(3)).find(|&i| &data[i..i + 4] == b"\r\n\r\n")
}

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
