//! HTTP download state — sends request, receives response, streams to disk.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::Socket as TcpSocket;
use smoltcp::time::Instant;

use crate::driver::traits::NetworkDriver;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{DoneState, FailedState};

/// HTTP download phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPhase {
    SendRequest,
    ReceiveHeaders,
    ReceiveBody,
    Complete,
}

/// HTTP state — handles HTTP/1.1 GET request and response.
///
/// # Standalone Usage
/// ```ignore
/// let http_state = HttpState::with_request(tcp_handle, "GET", "/path", "host");
/// ```
pub struct HttpState {
    tcp_handle: SocketHandle,
    phase: HttpPhase,
    start_tsc: u64,
    last_activity_tsc: u64,
    
    /// Request components (for standalone use)
    method: &'static str,
    path: Option<&'static str>,
    host: Option<&'static str>,
    
    /// Response parsing state
    headers_complete: bool,
    content_length: Option<u64>,
    chunked: bool,
    bytes_received: u64,
    
    /// Header parsing buffer
    header_buf: [u8; 2048],
    header_len: usize,
}

impl HttpState {
    /// Create HTTP state for download (uses path/host from context).
    pub fn new(tcp_handle: SocketHandle) -> Self {
        Self {
            tcp_handle,
            phase: HttpPhase::SendRequest,
            start_tsc: 0,
            last_activity_tsc: 0,
            method: "GET",
            path: None,
            host: None,
            headers_complete: false,
            content_length: None,
            chunked: false,
            bytes_received: 0,
            header_buf: [0u8; 2048],
            header_len: 0,
        }
    }

    /// Create HTTP state with explicit request (for standalone use).
    pub fn with_request(
        tcp_handle: SocketHandle,
        method: &'static str,
        path: &'static str,
        host: &'static str,
    ) -> Self {
        Self {
            tcp_handle,
            phase: HttpPhase::SendRequest,
            start_tsc: 0,
            last_activity_tsc: 0,
            method,
            path: Some(path),
            host: Some(host),
            headers_complete: false,
            content_length: None,
            chunked: false,
            bytes_received: 0,
            header_buf: [0u8; 2048],
            header_len: 0,
        }
    }

    /// Get current phase.
    pub fn phase(&self) -> HttpPhase {
        self.phase
    }

    /// Get bytes received so far.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    /// Get content length (if known from headers).
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }
}

impl<D: NetworkDriver> State<D> for HttpState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        adapter: &mut SmoltcpAdapter<'_, D>,
        now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        // Initialize on first call
        if self.start_tsc == 0 {
            self.start_tsc = tsc;
            self.last_activity_tsc = tsc;
            serial::println("[HTTP] Starting HTTP request...");
        }

        // Check idle timeout
        let idle_ticks = tsc.saturating_sub(self.last_activity_tsc);
        let idle_timeout = ctx.timeouts.http_idle();
        if idle_ticks > idle_timeout {
            serial::println("[HTTP] ERROR: Idle timeout");
            return (Box::new(FailedState::new("HTTP idle timeout")), StepResult::Failed("idle timeout"));
        }

        let socket = sockets.get_mut::<TcpSocket>(self.tcp_handle);

        match self.phase {
            HttpPhase::SendRequest => {
                if !socket.may_send() {
                    return (self, StepResult::Continue);
                }

                // Build request
                let path = self.path.unwrap_or(ctx.url_path);
                let host = self.host.unwrap_or(ctx.url_host);

                let mut req_buf = [0u8; 512];
                let req_len = format_http_request(&mut req_buf, self.method, path, host);

                if req_len == 0 {
                    serial::println("[HTTP] ERROR: Request too large");
                    return (Box::new(FailedState::new("request too large")), StepResult::Failed("request"));
                }

                serial::print("[HTTP] Sending ");
                serial::print(self.method);
                serial::print(" ");
                serial::println(path);

                if socket.send_slice(&req_buf[..req_len]).is_err() {
                    serial::println("[HTTP] ERROR: Send failed");
                    return (Box::new(FailedState::new("send failed")), StepResult::Failed("send"));
                }

                self.phase = HttpPhase::ReceiveHeaders;
                self.last_activity_tsc = tsc;
            }

            HttpPhase::ReceiveHeaders => {
                if !socket.may_recv() {
                    if socket.state() != smoltcp::socket::tcp::State::Established {
                        serial::println("[HTTP] ERROR: Connection closed during headers");
                        return (Box::new(FailedState::new("connection closed")), StepResult::Failed("closed"));
                    }
                    return (self, StepResult::Continue);
                }

                // Read into header buffer
                let space = self.header_buf.len() - self.header_len;
                if space == 0 {
                    serial::println("[HTTP] ERROR: Headers too large");
                    return (Box::new(FailedState::new("headers too large")), StepResult::Failed("headers"));
                }

                match socket.recv_slice(&mut self.header_buf[self.header_len..]) {
                    Ok(0) => {}
                    Ok(n) => {
                        self.header_len += n;
                        self.last_activity_tsc = tsc;

                        // Look for end of headers
                        if let Some(end) = find_header_end(&self.header_buf[..self.header_len]) {
                            // Parse headers
                            let header_str = core::str::from_utf8(&self.header_buf[..end])
                                .unwrap_or("");

                            // Check status
                            if !header_str.starts_with("HTTP/1.1 200") 
                                && !header_str.starts_with("HTTP/1.0 200") {
                                serial::print("[HTTP] ERROR: Bad status: ");
                                if let Some(line_end) = header_str.find('\r') {
                                    serial::println(&header_str[..line_end]);
                                }
                                return (Box::new(FailedState::new("bad HTTP status")), StepResult::Failed("status"));
                            }

                            serial::println("[HTTP] Got 200 OK");

                            // Parse Content-Length
                            self.content_length = parse_content_length(header_str);
                            if let Some(len) = self.content_length {
                                serial::print("[HTTP] Content-Length: ");
                                serial::print_u32((len / 1024 / 1024) as u32);
                                serial::println(" MB");
                                ctx.content_length = Some(len);
                            }

                            // Check for chunked encoding
                            self.chunked = header_str.to_ascii_lowercase()
                                .contains("transfer-encoding: chunked");

                            // Move body data to start of buffer
                            let body_start = end + 4; // Skip \r\n\r\n
                            let body_len = self.header_len - body_start;
                            if body_len > 0 {
                                // Process initial body data
                                self.bytes_received += body_len as u64;
                                ctx.bytes_downloaded = self.bytes_received;
                                // TODO: Write to disk if enabled
                            }

                            self.phase = HttpPhase::ReceiveBody;
                            serial::println("[HTTP] Receiving body...");
                        }
                    }
                    Err(_) => {}
                }
            }

            HttpPhase::ReceiveBody => {
                if !socket.may_recv() {
                    // Check if we're done
                    if let Some(expected) = self.content_length {
                        if self.bytes_received >= expected {
                            serial::println("[HTTP] Download complete");
                            self.phase = HttpPhase::Complete;
                            ctx.bytes_downloaded = self.bytes_received;
                            return (Box::new(DoneState::new()), StepResult::Transition);
                        }
                    }

                    // Connection closed?
                    if socket.state() != smoltcp::socket::tcp::State::Established {
                        if self.content_length.is_none() {
                            // No Content-Length, connection close = end
                            serial::println("[HTTP] Download complete (connection closed)");
                            ctx.bytes_downloaded = self.bytes_received;
                            return (Box::new(DoneState::new()), StepResult::Transition);
                        }
                        serial::println("[HTTP] ERROR: Premature connection close");
                        return (Box::new(FailedState::new("premature close")), StepResult::Failed("close"));
                    }
                    return (self, StepResult::Continue);
                }

                // Read body data
                let mut buf = [0u8; 4096];
                match socket.recv_slice(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        self.bytes_received += n as u64;
                        self.last_activity_tsc = tsc;
                        ctx.bytes_downloaded = self.bytes_received;

                        // Progress every 1MB
                        let mb = self.bytes_received / (1024 * 1024);
                        let prev_mb = (self.bytes_received - n as u64) / (1024 * 1024);
                        if mb > prev_mb {
                            serial::print("[HTTP] Downloaded: ");
                            serial::print_u32(mb as u32);
                            if let Some(total) = self.content_length {
                                serial::print("/");
                                serial::print_u32((total / 1024 / 1024) as u32);
                            }
                            serial::println(" MB");
                        }

                        // TODO: Write to disk if enabled
                        // This would use ctx.blk_device
                    }
                    Err(_) => {}
                }
            }

            HttpPhase::Complete => {
                return (Box::new(DoneState::new()), StepResult::Transition);
            }
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "HTTP"
    }
}

/// Format HTTP GET request into buffer. Returns length or 0 if buffer too small.
fn format_http_request(buf: &mut [u8], method: &str, path: &str, host: &str) -> usize {
    let mut pos = 0;

    // "{METHOD} {path} HTTP/1.1\r\nHost: {host}\r\n..."
    let parts: &[&[u8]] = &[
        method.as_bytes(),
        b" ",
        path.as_bytes(),
        b" HTTP/1.1\r\nHost: ",
        host.as_bytes(),
        b"\r\nUser-Agent: MorpheusX/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n",
    ];

    for part in parts {
        if pos + part.len() > buf.len() {
            return 0;
        }
        buf[pos..pos + part.len()].copy_from_slice(part);
        pos += part.len();
    }

    pos
}

/// Find end of HTTP headers (double CRLF).
fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

/// Parse Content-Length from headers.
fn parse_content_length(headers: &str) -> Option<u64> {
    for line in headers.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("content-length:") {
            let value = line[15..].trim();
            return value.parse().ok();
        }
    }
    None
}
