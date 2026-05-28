//! Non-blocking streaming HTTP GET, composing `DnsResolveState` and
//! `TcpConnState`. Body bytes flow to a callback as they arrive rather than
//! buffering the whole response.
//!
//! Init -> Resolving -> Connecting -> SendingRequest -> ReceivingHeaders ->
//! ReceivingBody -> Done, with Failed reachable from each active state.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::net::Ipv4Addr;

use super::dns::{resolve_without_dns, DnsError, DnsResolveState};
use super::tcp::{TcpConnState, TcpError, TcpSocketState};
use super::{StateError, StepResult, TscTimestamp};
use crate::http::Headers;
use crate::url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    InvalidUrl,
    DnsError(DnsError),
    TcpError(TcpError),
    SendTimeout,
    ReceiveTimeout,
    HttpStatus { code: u16, reason: String },
    InvalidResponse,
    ResponseTooLarge,
    ConnectionClosed,
    HttpsNotSupported,
}

crate::impl_from!(DnsError => HttpError : DnsError);
crate::impl_from!(TcpError => HttpError : TcpError);

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

#[derive(Debug, Clone)]
pub struct HttpResponseInfo {
    pub status_code: u16,
    pub reason: String,
    pub content_length: Option<usize>,
    pub content_type: Option<String>,
    pub chunked: bool,
    pub headers: Headers,
}

impl HttpResponseInfo {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status_code)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HttpProgress {
    pub received: usize,
    /// Set only if Content-Length is known.
    pub total: Option<usize>,
}

impl HttpProgress {
    /// 0-100.
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

#[derive(Debug)]
pub struct HeaderAccumulator {
    buffer: Vec<u8>,
    /// Caps header size to bound memory.
    max_size: usize,
}

impl HeaderAccumulator {
    const DEFAULT_MAX_SIZE: usize = 16 * 1024;

    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            max_size: Self::DEFAULT_MAX_SIZE,
        }
    }

    /// Returns true once the \r\n\r\n terminator is present.
    fn append(&mut self, data: &[u8]) -> Result<bool, HttpError> {
        if self.buffer.len() + data.len() > self.max_size {
            return Err(HttpError::ResponseTooLarge);
        }

        self.buffer.extend_from_slice(data);

        Ok(self.find_header_end().is_some())
    }

    fn find_header_end(&self) -> Option<usize> {
        self.buffer.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn parse(&self) -> Result<(HttpResponseInfo, &[u8]), HttpError> {
        let sep_pos = self.find_header_end().ok_or(HttpError::InvalidResponse)?;

        let header_bytes = &self.buffer[..sep_pos];
        let body_bytes = &self.buffer[sep_pos + 4..];

        let info = Self::parse_headers(header_bytes)?;

        Ok((info, body_bytes))
    }

    fn parse_headers(data: &[u8]) -> Result<HttpResponseInfo, HttpError> {
        let header_str = core::str::from_utf8(data).map_err(|_| HttpError::InvalidResponse)?;

        let mut lines = header_str.lines();

        let status_line = lines.next().ok_or(HttpError::InvalidResponse)?;

        let (status_code, reason) = Self::parse_status_line(status_line)?;

        let mut headers = Headers::new();
        for line in lines {
            if line.is_empty() {
                break;
            }

            if let Some((name, value)) = line.split_once(':') {
                headers.set(name.trim(), value.trim());
            }
        }

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

    /// Parse a status line like "HTTP/1.1 200 OK".
    fn parse_status_line(line: &str) -> Result<(u16, String), HttpError> {
        let mut parts = line.split_whitespace();

        let _version = parts.next().ok_or(HttpError::InvalidResponse)?;

        let code_str = parts.next().ok_or(HttpError::InvalidResponse)?;
        let code = code_str.parse().map_err(|_| HttpError::InvalidResponse)?;

        let reason: String = parts.collect::<Vec<_>>().join(" ");
        let reason = if reason.is_empty() {
            Self::default_reason(code).to_string()
        } else {
            reason
        };

        Ok((code, reason))
    }

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

/// Orchestrates DNS -> TCP -> HTTP request/response, streaming the body.
#[derive(Debug)]
pub(crate) enum HttpDownloadState {
    Init {
        url: Url,
    },
    Resolving {
        dns: DnsResolveState,
        host: String,
        port: u16,
        path: String,
        query: Option<String>,
    },
    Connecting {
        tcp: TcpConnState,
        ip: Ipv4Addr,
        port: u16,
        host_header: String,
        request_uri: String,
    },
    SendingRequest {
        socket_handle: usize,
        request: Vec<u8>,
        sent: usize,
        start_tsc: TscTimestamp,
    },
    ReceivingHeaders {
        socket_handle: usize,
        accumulator: HeaderAccumulator,
        start_tsc: TscTimestamp,
    },
    ReceivingBody {
        socket_handle: usize,
        response_info: HttpResponseInfo,
        received: usize,
        start_tsc: TscTimestamp,
        /// Reset on each chunk; drives the idle timeout.
        last_activity_tsc: TscTimestamp,
    },
    Done {
        response_info: HttpResponseInfo,
        /// Body bytes only.
        total_bytes: usize,
    },
    Failed {
        error: HttpError,
    },
}

impl HttpDownloadState {
    pub fn new(url: Url) -> Result<Self, HttpError> {
        // No TLS in this bare-metal stack.
        if url.is_https() {
            return Err(HttpError::HttpsNotSupported);
        }

        Ok(HttpDownloadState::Init { url })
    }

    /// Transition Init -> Resolving, or -> Connecting if the IP is already known.
    pub fn start(&mut self, _now_tsc: u64) {
        if let HttpDownloadState::Init { url } = self {
            let host = url.host.clone();
            let port = url.port_or_default();
            let path = url.path.clone();
            let query = url.query.clone();
            let host_header = url.host_header();
            let request_uri = url.request_uri();

            // Connection/query initiation itself happens in step().
            if let Some(ip) = resolve_without_dns(&host) {
                let tcp = TcpConnState::new();
                *self = HttpDownloadState::Connecting {
                    tcp,
                    ip,
                    port,
                    host_header,
                    request_uri,
                };
            } else {
                let dns = DnsResolveState::new();
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

    /// In ReceivingBody, the caller must forward `recv_data` to the storage
    /// layer; this machine only tracks counts.
    pub fn step(
        &mut self,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        _can_send: bool,
        now_tsc: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> StepResult {
        // Move out so transitions can consume the current state.
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
            _can_send,
            now_tsc,
            dns_timeout,
            tcp_timeout,
            http_send_timeout,
            http_recv_timeout,
        );

        *self = new_state;
        result
    }

    fn step_inner(
        &self,
        current: HttpDownloadState,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        _can_send: bool,
        now_tsc: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> (HttpDownloadState, StepResult) {
        match current {
            HttpDownloadState::Init { url } => {
                (HttpDownloadState::Init { url }, StepResult::Pending)
            },

            HttpDownloadState::Resolving {
                mut dns,
                host,
                port,
                path,
                query,
            } => {
                let result = dns.step(dns_result, now_tsc, dns_timeout);

                match result {
                    StepResult::Done => {
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
                    },
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
                    },
                }
            },

            HttpDownloadState::Connecting {
                mut tcp,
                ip,
                port,
                host_header,
                request_uri,
            } => {
                let result = tcp.step(tcp_state, now_tsc, tcp_timeout);

                match result {
                    StepResult::Done => {
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
                    },
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
                    },
                }
            },

            HttpDownloadState::SendingRequest {
                socket_handle,
                request,
                sent,
                start_tsc,
            } => {
                if start_tsc.is_expired(now_tsc, http_send_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::SendTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

                if tcp_state == TcpSocketState::Closed {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ConnectionClosed,
                        },
                        StepResult::Failed,
                    );
                }

                // The actual send is driven externally; we only track progress.
                if sent >= request.len() {
                    (
                        HttpDownloadState::ReceivingHeaders {
                            socket_handle,
                            accumulator: HeaderAccumulator::new(),
                            start_tsc: TscTimestamp::new(now_tsc),
                        },
                        StepResult::Pending,
                    )
                } else {
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
            },

            HttpDownloadState::ReceivingHeaders {
                socket_handle,
                mut accumulator,
                start_tsc,
            } => {
                if start_tsc.is_expired(now_tsc, http_recv_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ReceiveTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

                if tcp_state == TcpSocketState::Closed {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ConnectionClosed,
                        },
                        StepResult::Failed,
                    );
                }

                if let Some(data) = recv_data {
                    match accumulator.append(data) {
                        Ok(true) => {
                            match accumulator.parse() {
                                Ok((response_info, body_data)) => {
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

                                    // body_data may already hold leading body bytes.
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
                                },
                                Err(e) => {
                                    (HttpDownloadState::Failed { error: e }, StepResult::Failed)
                                },
                            }
                        },
                        Ok(false) => (
                            HttpDownloadState::ReceivingHeaders {
                                socket_handle,
                                accumulator,
                                start_tsc,
                            },
                            StepResult::Pending,
                        ),
                        Err(e) => (HttpDownloadState::Failed { error: e }, StepResult::Failed),
                    }
                } else {
                    (
                        HttpDownloadState::ReceivingHeaders {
                            socket_handle,
                            accumulator,
                            start_tsc,
                        },
                        StepResult::Pending,
                    )
                }
            },

            HttpDownloadState::ReceivingBody {
                socket_handle,
                response_info,
                received,
                start_tsc,
                last_activity_tsc,
            } => {
                if last_activity_tsc.is_expired(now_tsc, http_recv_timeout) {
                    return (
                        HttpDownloadState::Failed {
                            error: HttpError::ReceiveTimeout,
                        },
                        StepResult::Timeout,
                    );
                }

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

                if tcp_state == TcpSocketState::Closed {
                    // No Content-Length: close signals EOF; otherwise it is truncated.
                    if response_info.content_length.is_none() {
                        return (
                            HttpDownloadState::Done {
                                response_info,
                                total_bytes: received,
                            },
                            StepResult::Done,
                        );
                    } else {
                        return (
                            HttpDownloadState::Failed {
                                error: HttpError::ConnectionClosed,
                            },
                            StepResult::Failed,
                        );
                    }
                }

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
            },

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
            },
        }
    }

    fn build_request(host: &str, request_uri: &str) -> Vec<u8> {
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

    /// Unsent request bytes and the offset already sent (SendingRequest only).
    pub fn request_bytes(&self) -> Option<(&[u8], usize)> {
        if let HttpDownloadState::SendingRequest { request, sent, .. } = self {
            Some((&request[*sent..], *sent))
        } else {
            None
        }
    }

    pub fn mark_sent(&mut self, additional: usize) {
        if let HttpDownloadState::SendingRequest { sent, .. } = self {
            *sent += additional;
        }
    }

    pub fn socket_handle(&self) -> Option<usize> {
        match self {
            HttpDownloadState::SendingRequest { socket_handle, .. }
            | HttpDownloadState::ReceivingHeaders { socket_handle, .. }
            | HttpDownloadState::ReceivingBody { socket_handle, .. } => Some(*socket_handle),
            HttpDownloadState::Connecting { tcp, .. } => tcp.socket_handle(),
            _ => None,
        }
    }

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

    pub fn response_info(&self) -> Option<&HttpResponseInfo> {
        match self {
            HttpDownloadState::ReceivingBody { response_info, .. }
            | HttpDownloadState::Done { response_info, .. } => Some(response_info),
            _ => None,
        }
    }

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

    pub fn error(&self) -> Option<&HttpError> {
        if let HttpDownloadState::Failed { error } = self {
            Some(error)
        } else {
            None
        }
    }

    /// Done or failed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            HttpDownloadState::Done { .. } | HttpDownloadState::Failed { .. }
        )
    }

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
