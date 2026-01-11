//! HTTP download state machine.
//!
//! Non-blocking HTTP GET with streaming support for ISO downloads.
//!
//! # States
//! ```text
//! Init → Resolving → Connecting → SendingRequest → ReceivingHeaders → ReceivingBody → Done
//!   ↓         ↓           ↓              ↓                ↓                 ↓
//! Failed   Failed      Failed         Failed           Failed           Failed
//! ```
//!
//! # Architecture
//!
//! This state machine composes DNS and TCP state machines:
//! - `DnsResolveState` for hostname resolution
//! - `TcpConnState` for connection establishment
//!
//! The HTTP layer handles request/response framing on top of TCP.
//!
//! # Streaming Mode
//!
//! For large downloads (ISOs), data is passed to a callback function
//! as it arrives, rather than buffering the entire response.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.5

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::net::Ipv4Addr;

use super::dns::{resolve_without_dns, DnsError, DnsResolveState};
use super::tcp::{TcpConnState, TcpError, TcpSocketState};
use super::{StateError, StepResult, TscTimestamp};
use crate::http::Headers;
use crate::url::Url;

// ═══════════════════════════════════════════════════════════════════════════
// HTTP ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// HTTP-specific errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Invalid URL
    InvalidUrl,
    /// DNS resolution failed
    DnsError(DnsError),
    /// TCP connection failed
    TcpError(TcpError),
    /// Send timeout
    SendTimeout,
    /// Receive timeout
    ReceiveTimeout,
    /// HTTP status error (non-2xx)
    HttpStatus { code: u16, reason: String },
    /// Invalid HTTP response
    InvalidResponse,
    /// Response too large
    ResponseTooLarge,
    /// Connection closed unexpectedly
    ConnectionClosed,
    /// HTTPS not supported
    HttpsNotSupported,
}

impl From<DnsError> for HttpError {
    fn from(e: DnsError) -> Self {
        HttpError::DnsError(e)
    }
}

impl From<TcpError> for HttpError {
    fn from(e: TcpError) -> Self {
        HttpError::TcpError(e)
    }
}

impl From<HttpError> for StateError {
    fn from(e: HttpError) -> Self {
        match e {
            HttpError::DnsError(_) => StateError::DnsError,
            HttpError::TcpError(TcpError::ConnectTimeout) => StateError::Timeout,
            HttpError::TcpError(TcpError::ConnectionRefused) => StateError::ConnectionRefused,
            HttpError::TcpError(TcpError::ConnectionReset) => StateError::ConnectionReset,
            HttpError::TcpError(_) => StateError::ConnectionFailed,
            HttpError::SendTimeout | HttpError::ReceiveTimeout => StateError::Timeout,
            HttpError::HttpStatus { code, .. } => StateError::HttpStatus(code),
            _ => StateError::HttpError,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HTTP RESPONSE INFO
// ═══════════════════════════════════════════════════════════════════════════

/// Information extracted from HTTP response headers.
#[derive(Debug, Clone)]
pub struct HttpResponseInfo {
    /// HTTP status code (e.g., 200, 404)
    pub status_code: u16,
    /// Reason phrase (e.g., "OK", "Not Found")
    pub reason: String,
    /// Content-Length if provided
    pub content_length: Option<usize>,
    /// Content-Type if provided
    pub content_type: Option<String>,
    /// Whether response uses chunked transfer encoding
    pub chunked: bool,
    /// Full headers
    pub headers: Headers,
}

impl HttpResponseInfo {
    /// Check if response indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    /// Check if response indicates redirect (3xx).
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status_code)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HTTP DOWNLOAD PROGRESS
// ═══════════════════════════════════════════════════════════════════════════

/// Download progress information.
#[derive(Debug, Clone, Copy)]
pub struct HttpProgress {
    /// Bytes received so far
    pub received: usize,
    /// Total expected bytes (if Content-Length known)
    pub total: Option<usize>,
}

impl HttpProgress {
    /// Calculate percentage complete (0-100).
    pub fn percent(&self) -> Option<u8> {
        self.total.map(|t| {
            if t == 0 {
                100
            } else {
                ((self.received as u64 * 100) / t as u64) as u8
            }
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HEADER PARSING STATE
// ═══════════════════════════════════════════════════════════════════════════

/// Internal state for accumulating headers.
#[derive(Debug)]
struct HeaderAccumulator {
    /// Raw header data
    buffer: Vec<u8>,
    /// Maximum header size (prevent DoS)
    max_size: usize,
}

impl HeaderAccumulator {
    const DEFAULT_MAX_SIZE: usize = 16 * 1024; // 16KB max headers

    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            max_size: Self::DEFAULT_MAX_SIZE,
        }
    }

    /// Append data to buffer.
    /// Returns true if headers complete (\r\n\r\n found).
    fn append(&mut self, data: &[u8]) -> Result<bool, HttpError> {
        if self.buffer.len() + data.len() > self.max_size {
            return Err(HttpError::ResponseTooLarge);
        }

        self.buffer.extend_from_slice(data);

        // Check for end of headers: \r\n\r\n
        Ok(self.find_header_end().is_some())
    }

    /// Find position of header/body separator.
    fn find_header_end(&self) -> Option<usize> {
        self.buffer.windows(4).position(|w| w == b"\r\n\r\n")
    }

    /// Parse headers and return body data.
    fn parse(&self) -> Result<(HttpResponseInfo, &[u8]), HttpError> {
        let sep_pos = self.find_header_end().ok_or(HttpError::InvalidResponse)?;

        let header_bytes = &self.buffer[..sep_pos];
        let body_bytes = &self.buffer[sep_pos + 4..];

        let info = Self::parse_headers(header_bytes)?;

        Ok((info, body_bytes))
    }

    /// Parse HTTP response headers.
    fn parse_headers(data: &[u8]) -> Result<HttpResponseInfo, HttpError> {
        let header_str = core::str::from_utf8(data).map_err(|_| HttpError::InvalidResponse)?;

        let mut lines = header_str.lines();

        // Parse status line: "HTTP/1.1 200 OK"
        let status_line = lines.next().ok_or(HttpError::InvalidResponse)?;

        let (status_code, reason) = Self::parse_status_line(status_line)?;

        // Parse headers
        let mut headers = Headers::new();
        for line in lines {
            if line.is_empty() {
                break;
            }

            if let Some((name, value)) = line.split_once(':') {
                headers.set(name.trim(), value.trim());
            }
        }

        // Extract key header values
        let content_length = headers.get("Content-Length").and_then(|v| v.parse().ok());

        let content_type = headers.get("Content-Type").map(|v| v.to_string());

        let chunked = headers
            .get("Transfer-Encoding")
            .map(|v| v.eq_ignore_ascii_case("chunked"))
            .unwrap_or(false);

        Ok(HttpResponseInfo {
            status_code,
            reason,
            content_length,
            content_type,
            chunked,
            headers,
        })
    }

    /// Parse HTTP status line.
    fn parse_status_line(line: &str) -> Result<(u16, String), HttpError> {
        // "HTTP/1.1 200 OK"
        let mut parts = line.split_whitespace();

        // HTTP version
        let _version = parts.next().ok_or(HttpError::InvalidResponse)?;

        // Status code
        let code_str = parts.next().ok_or(HttpError::InvalidResponse)?;
        let code = code_str.parse().map_err(|_| HttpError::InvalidResponse)?;

        // Reason phrase (rest of line)
        let reason: String = parts.collect::<Vec<_>>().join(" ");
        let reason = if reason.is_empty() {
            Self::default_reason(code).to_string()
        } else {
            reason
        };

        Ok((code, reason))
    }

    /// Default reason phrase for status code.
    fn default_reason(code: u16) -> &'static str {
        match code {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            206 => "Partial Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            500 => "Internal Server Error",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            _ => "Unknown",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HTTP DOWNLOAD STATE MACHINE
// ═══════════════════════════════════════════════════════════════════════════

/// HTTP download state machine.
///
/// Orchestrates DNS resolution → TCP connection → HTTP request/response.
/// Supports streaming for large downloads.
#[derive(Debug)]
pub enum HttpDownloadState {
    /// Initial state with target URL.
    Init { url: Url },

    /// Resolving hostname to IP address.
    Resolving {
        /// DNS state machine
        dns: DnsResolveState,
        /// Target host
        host: String,
        /// Target port
        port: u16,
        /// Request path
        path: String,
        /// Query string (if any)
        query: Option<String>,
    },

    /// Connecting to server.
    Connecting {
        /// TCP state machine
        tcp: TcpConnState,
        /// Resolved IP
        ip: Ipv4Addr,
        /// Target port
        port: u16,
        /// Host header value
        host_header: String,
        /// Request path + query
        request_uri: String,
    },

    /// Sending HTTP request.
    SendingRequest {
        /// Socket handle
        socket_handle: usize,
        /// Serialized HTTP request
        request: Vec<u8>,
        /// Bytes sent so far
        sent: usize,
        /// When send started
        start_tsc: TscTimestamp,
    },

    /// Receiving HTTP response headers.
    ReceivingHeaders {
        /// Socket handle
        socket_handle: usize,
        /// Header accumulator
        accumulator: HeaderAccumulator,
        /// When receive started
        start_tsc: TscTimestamp,
    },

    /// Receiving HTTP response body.
    ReceivingBody {
        /// Socket handle
        socket_handle: usize,
        /// Response info from headers
        response_info: HttpResponseInfo,
        /// Bytes received so far
        received: usize,
        /// When body receive started
        start_tsc: TscTimestamp,
        /// Last activity time (for idle timeout)
        last_activity_tsc: TscTimestamp,
    },

    /// Download complete.
    Done {
        /// Response info
        response_info: HttpResponseInfo,
        /// Total bytes received (body only)
        total_bytes: usize,
    },

    /// Download failed.
    Failed {
        /// Error details
        error: HttpError,
    },
}

impl HttpDownloadState {
    /// Create new HTTP download state machine.
    pub fn new(url: Url) -> Result<Self, HttpError> {
        // HTTPS not supported in this bare-metal implementation
        if url.is_https() {
            return Err(HttpError::HttpsNotSupported);
        }

        Ok(HttpDownloadState::Init { url })
    }

    /// Start the download.
    ///
    /// Transitions from Init to Resolving (or Connecting if IP known).
    pub fn start(&mut self, now_tsc: u64) {
        if let HttpDownloadState::Init { url } = self {
            let host = url.host.clone();
            let port = url.port_or_default();
            let path = url.path.clone();
            let query = url.query.clone();
            let host_header = url.host_header();
            let request_uri = url.request_uri();

            // Try to resolve without DNS first (IP or hardcoded)
            if let Some(ip) = resolve_without_dns(&host) {
                // Skip DNS, go directly to connecting
                let mut tcp = TcpConnState::new();
                // Note: actual connection initiation happens in step()
                *self = HttpDownloadState::Connecting {
                    tcp,
                    ip,
                    port,
                    host_header,
                    request_uri,
                };
            } else {
                // Need DNS resolution
                let mut dns = DnsResolveState::new();
                // Note: actual DNS query initiation happens in step()
                *self = HttpDownloadState::Resolving {
                    dns,
                    host,
                    port,
                    path,
                    query,
                };
            }
        }
    }

    /// Step the state machine.
    ///
    /// # Arguments
    /// - `dns_result`: Result from smoltcp DNS query (if resolving)
    /// - `tcp_state`: Current TCP socket state (if connected)
    /// - `recv_data`: Data received from socket (if any)
    /// - `can_send`: Whether socket can accept more data
    /// - `now_tsc`: Current TSC value
    /// - `timeouts`: Timeout configuration (DNS, TCP, HTTP timeouts)
    ///
    /// # Returns
    /// - `Pending`: Still in progress
    /// - `Done`: Download complete, call `result()` for info
    /// - `Timeout`: Operation timed out
    /// - `Failed`: Operation failed, call `error()` for details
    ///
    /// # Callback
    ///
    /// When `recv_data` contains body data during ReceivingBody state,
    /// the caller should pass that data to the storage layer.
    pub fn step(
        &mut self,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        can_send: bool,
        now_tsc: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> StepResult {
        // Take ownership for state transitions
        let current = core::mem::replace(
            self,
            HttpDownloadState::Init {
                url: Url {
                    scheme: crate::url::parser::Scheme::Http,
                    host: String::new(),
                    port: None,
                    path: String::new(),
                    query: None,
                },
            },
        );

        let (new_state, result) = self.step_inner(
            current,
            dns_result,
            tcp_state,
            recv_data,
            can_send,
            now_tsc,
            dns_timeout,
            tcp_timeout,
            http_send_timeout,
            http_recv_timeout,
        );

        *self = new_state;
        result
    }

    /// Internal step implementation.
    fn step_inner(
        &self,
        current: HttpDownloadState,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        can_send: bool,
        now_tsc: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> (HttpDownloadState, StepResult) {
        match current {
            HttpDownloadState::Init { url } => {
                // Not started yet
                (HttpDownloadState::Init { url }, StepResult::Pending)
            }

            HttpDownloadState::Resolving {
                mut dns,
                host,
                port,
                path,
                query,
            } => {
                // Step DNS state machine
                let result = dns.step(dns_result, now_tsc, dns_timeout);

                match result {
                    StepResult::Done => {
                        // DNS resolved, start TCP connection
                        let ip = dns.ip().unwrap();
                        let host_header = if port != 80 {
                            alloc::format!("{}:{}", host, port)
                        } else {
                            host.clone()
                        };
                        let request_uri = match &query {
                            Some(q) => alloc::format!("{}?{}", path, q),
                            None => path.clone(),
                        };

                        (
                            HttpDownloadState::Connecting {
                                tcp: TcpConnState::new(),
                                ip,
                                port,
                                host_header,
                                request_uri,
                            },
                            StepResult::Pending,
                        )
                    }
                    StepResult::Pending => (
                        HttpDownloadState::Resolving {
                            dns,
                            host,
                            port,
                            path,
                            query,
                        },
                        StepResult::Pending,
                    ),
                    StepResult::Timeout => (
                        HttpDownloadState::Failed {
                            error: HttpError::DnsError(DnsError::Timeout),
                        },
                        StepResult::Timeout,
                    ),
                    StepResult::Failed => {
                        let error = dns.error().unwrap_or(DnsError::QueryFailed);
                        (
                            HttpDownloadState::Failed {
                                error: HttpError::DnsError(error),
                            },
                            StepResult::Failed,
                        )
                    }
                }
            }

            HttpDownloadState::Connecting {
                mut tcp,
                ip,
                port,
                host_header,
                request_uri,
            } => {
                // Step TCP state machine
                let result = tcp.step(tcp_state, now_tsc, tcp_timeout);

                match result {
                    StepResult::Done => {
                        // Connected! Build HTTP request
                        let socket_handle = tcp.socket_handle().unwrap();
                        let request = Self::build_request(&host_header, &request_uri);

                        (
                            HttpDownloadState::SendingRequest {
                                socket_handle,
                                request,
                                sent: 0,
                                start_tsc: TscTimestamp::new(now_tsc),
                            },
                            StepResult::Pending,
                        )
                    }
                    StepResult::Pending => (
                        HttpDownloadState::Connecting {
                            tcp,
                            ip,
                            port,
                            host_header,
                            request_uri,
                        },
                        StepResult::Pending,
                    ),
                    StepResult::Timeout => (
                        HttpDownloadState::Failed {
                            error: HttpError::TcpError(TcpError::ConnectTimeout),
                        },
                        StepResult::Timeout,
                    ),
                    StepResult::Failed => {
                        let error = tcp.error().unwrap_or(TcpError::ConnectionRefused);
                        (
                            HttpDownloadState::Failed {
                                error: HttpError::TcpError(error),
                            },
                            StepResult::Failed,
                        )
                    }
                }
            }

            HttpDownloadState::SendingRequest {
                socket_handle,
                request,
                sent,
                start_tsc,
            } => {
                // Check timeout
                if start_tsc.is_expired(now_tsc, http_send_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::SendTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

                // Check connection status
                if tcp_state == TcpSocketState::Closed {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ConnectionClosed,
                        },
                        StepResult::Failed,
                    );
                }

                // Sending is tracked externally, we just track progress
                if sent >= request.len() {
                    // Request fully sent, start receiving
                    (
                        HttpDownloadState::ReceivingHeaders {
                            socket_handle,
                            accumulator: HeaderAccumulator::new(),
                            start_tsc: TscTimestamp::new(now_tsc),
                        },
                        StepResult::Pending,
                    )
                } else {
                    // Still sending
                    (
                        HttpDownloadState::SendingRequest {
                            socket_handle,
                            request,
                            sent,
                            start_tsc,
                        },
                        StepResult::Pending,
                    )
                }
            }

            HttpDownloadState::ReceivingHeaders {
                socket_handle,
                mut accumulator,
                start_tsc,
            } => {
                // Check timeout
                if start_tsc.is_expired(now_tsc, http_recv_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ReceiveTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

                // Check connection status
                if tcp_state == TcpSocketState::Closed {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ConnectionClosed,
                        },
                        StepResult::Failed,
                    );
                }

                // Process received data
                if let Some(data) = recv_data {
                    match accumulator.append(data) {
                        Ok(true) => {
                            // Headers complete, parse them
                            match accumulator.parse() {
                                Ok((response_info, body_data)) => {
                                    // Check for HTTP error status
                                    if !response_info.is_success() && !response_info.is_redirect() {
                                        return (
                                            HttpDownloadState::Failed {
                                                error: HttpError::HttpStatus {
                                                    code: response_info.status_code,
                                                    reason: response_info.reason.clone(),
                                                },
                                            },
                                            StepResult::Failed,
                                        );
                                    }

                                    // Start body reception
                                    // Note: body_data may contain initial body bytes
                                    let initial_received = body_data.len();

                                    (
                                        HttpDownloadState::ReceivingBody {
                                            socket_handle,
                                            response_info,
                                            received: initial_received,
                                            start_tsc: TscTimestamp::new(now_tsc),
                                            last_activity_tsc: TscTimestamp::new(now_tsc),
                                        },
                                        StepResult::Pending,
                                    )
                                }
                                Err(e) => {
                                    (HttpDownloadState::Failed { error: e }, StepResult::Failed)
                                }
                            }
                        }
                        Ok(false) => {
                            // Headers not complete yet
                            (
                                HttpDownloadState::ReceivingHeaders {
                                    socket_handle,
                                    accumulator,
                                    start_tsc,
                                },
                                StepResult::Pending,
                            )
                        }
                        Err(e) => (HttpDownloadState::Failed { error: e }, StepResult::Failed),
                    }
                } else {
                    // No data received yet
                    (
                        HttpDownloadState::ReceivingHeaders {
                            socket_handle,
                            accumulator,
                            start_tsc,
                        },
                        StepResult::Pending,
                    )
                }
            }

            HttpDownloadState::ReceivingBody {
                socket_handle,
                response_info,
                received,
                start_tsc,
                last_activity_tsc,
            } => {
                // Check for idle timeout (no data for too long)
                if last_activity_tsc.is_expired(now_tsc, http_recv_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ReceiveTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

                // Check if complete based on Content-Length
                if let Some(content_length) = response_info.content_length {
                    if received >= content_length {
                        return (
                            HttpDownloadState::Done {
                                response_info,
                                total_bytes: received,
                            },
                            StepResult::Done,
                        );
                    }
                }

                // Check connection status
                if tcp_state == TcpSocketState::Closed {
                    // Connection closed - check if we're done
                    if response_info.content_length.is_none() {
                        // No Content-Length, connection close means done
                        return (
                            HttpDownloadState::Done {
                                response_info,
                                total_bytes: received,
                            },
                            StepResult::Done,
                        );
                    } else {
                        // Had Content-Length but didn't receive it all
                        return (
                            HttpDownloadState::Failed {
                                error: HttpError::ConnectionClosed,
                            },
                            StepResult::Failed,
                        );
                    }
                }

                // Process received data
                let new_received = if let Some(data) = recv_data {
                    received + data.len()
                } else {
                    received
                };

                let new_last_activity = if recv_data.is_some() {
                    TscTimestamp::new(now_tsc)
                } else {
                    last_activity_tsc
                };

                (
                    HttpDownloadState::ReceivingBody {
                        socket_handle,
                        response_info,
                        received: new_received,
                        start_tsc,
                        last_activity_tsc: new_last_activity,
                    },
                    StepResult::Pending,
                )
            }

            HttpDownloadState::Done {
                response_info,
                total_bytes,
            } => (
                HttpDownloadState::Done {
                    response_info,
                    total_bytes,
                },
                StepResult::Done,
            ),

            HttpDownloadState::Failed { error } => {
                let result = match &error {
                    HttpError::SendTimeout
                    | HttpError::ReceiveTimeout
                    | HttpError::DnsError(DnsError::Timeout)
                    | HttpError::TcpError(TcpError::ConnectTimeout)
                    | HttpError::TcpError(TcpError::CloseTimeout) => StepResult::Timeout,
                    _ => StepResult::Failed,
                };
                (HttpDownloadState::Failed { error }, result)
            }
        }
    }

    /// Build HTTP GET request.
    fn build_request(host: &str, request_uri: &str) -> Vec<u8> {
        // Build HTTP/1.1 GET request
        let request = alloc::format!(
            "GET {} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: MorpheusX/1.0\r\n\
             Accept: */*\r\n\
             Connection: close\r\n\
             \r\n",
            request_uri,
            host
        );

        request.into_bytes()
    }

    /// Get the request bytes to send (for SendingRequest state).
    pub fn request_bytes(&self) -> Option<(&[u8], usize)> {
        if let HttpDownloadState::SendingRequest { request, sent, .. } = self {
            Some((&request[*sent..], *sent))
        } else {
            None
        }
    }

    /// Update bytes sent (for SendingRequest state).
    pub fn mark_sent(&mut self, additional: usize) {
        if let HttpDownloadState::SendingRequest { sent, .. } = self {
            *sent += additional;
        }
    }

    /// Get socket handle (if connected).
    pub fn socket_handle(&self) -> Option<usize> {
        match self {
            HttpDownloadState::SendingRequest { socket_handle, .. }
            | HttpDownloadState::ReceivingHeaders { socket_handle, .. }
            | HttpDownloadState::ReceivingBody { socket_handle, .. } => Some(*socket_handle),
            HttpDownloadState::Connecting { tcp, .. } => tcp.socket_handle(),
            _ => None,
        }
    }

    /// Get download progress.
    pub fn progress(&self) -> Option<HttpProgress> {
        if let HttpDownloadState::ReceivingBody {
            response_info,
            received,
            ..
        } = self
        {
            Some(HttpProgress {
                received: *received,
                total: response_info.content_length,
            })
        } else {
            None
        }
    }

    /// Get response info (if headers received).
    pub fn response_info(&self) -> Option<&HttpResponseInfo> {
        match self {
            HttpDownloadState::ReceivingBody { response_info, .. }
            | HttpDownloadState::Done { response_info, .. } => Some(response_info),
            _ => None,
        }
    }

    /// Get result (if complete).
    pub fn result(&self) -> Option<(&HttpResponseInfo, usize)> {
        if let HttpDownloadState::Done {
            response_info,
            total_bytes,
        } = self
        {
            Some((response_info, *total_bytes))
        } else {
            None
        }
    }

    /// Get error (if failed).
    pub fn error(&self) -> Option<&HttpError> {
        if let HttpDownloadState::Failed { error } = self {
            Some(error)
        } else {
            None
        }
    }

    /// Check if download is complete (success or failure).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            HttpDownloadState::Done { .. } | HttpDownloadState::Failed { .. }
        )
    }

    /// Check if download is in progress.
    pub fn is_active(&self) -> bool {
        !matches!(
            self,
            HttpDownloadState::Init { .. }
                | HttpDownloadState::Done { .. }
                | HttpDownloadState::Failed { .. }
        )
    }
}

impl Default for HttpDownloadState {
    fn default() -> Self {
        HttpDownloadState::Init {
            url: Url {
                scheme: crate::url::parser::Scheme::Http,
                host: String::new(),
                port: None,
                path: String::new(),
                query: None,
            },
        }
    }
}
