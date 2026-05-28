//! HTTP download state — sends request, receives response, streams to disk.

extern crate alloc;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::Socket as TcpSocket;
use smoltcp::time::Instant;

use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::disk_writer::DiskWriter;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};
use morpheus_nic::traits::NetworkDriver;

use super::{DoneState, FailedState, ManifestState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpPhase {
    SendRequest,
    ReceiveHeaders,
    ReceiveBody,
    Complete,
}

/// HTTP/1.1 GET request/response, optionally streaming the body to disk.
pub(crate) struct HttpState {
    tcp_handle: SocketHandle,
    phase: HttpPhase,
    start_tsc: u64,
    last_activity_tsc: u64,
    method: &'static str,
    path: Option<&'static str>,
    host: Option<&'static str>,
    headers_complete: bool,
    content_length: Option<u64>,
    chunked: bool,
    bytes_received: u64,
    header_buf: [u8; 2048],
    header_len: usize,
    disk_writer: Option<DiskWriter>,
}

impl HttpState {
    /// Path/host come from the context.
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
            disk_writer: None,
        }
    }

    pub fn with_disk_write(tcp_handle: SocketHandle, start_sector: u64) -> Self {
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
            disk_writer: Some(DiskWriter::new(start_sector)),
        }
    }

    /// Explicit method/path/host for standalone use.
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
            disk_writer: None,
        }
    }

    pub fn phase(&self) -> HttpPhase {
        self.phase
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }
}

impl<D: NetworkDriver> State<D> for HttpState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.start_tsc == 0 {
            self.start_tsc = tsc;
            self.last_activity_tsc = tsc;
            serial::println("[HTTP] Starting HTTP request...");
        }

        let idle_ticks = tsc.saturating_sub(self.last_activity_tsc);
        let idle_timeout = ctx.timeouts.http_idle();
        if idle_ticks > idle_timeout {
            serial::println("[HTTP] ERROR: Idle timeout");
            return (
                Box::new(FailedState::new("HTTP idle timeout")),
                StepResult::Failed("idle timeout"),
            );
        }

        let socket = sockets.get_mut::<TcpSocket>(self.tcp_handle);

        match self.phase {
            HttpPhase::SendRequest => {
                if !socket.may_send() {
                    return (self, StepResult::Continue);
                }

                let path = self.path.unwrap_or(ctx.url_path);
                let host = self.host.unwrap_or(ctx.url_host);

                let mut req_buf = [0u8; 512];
                let req_len = format_http_request(&mut req_buf, self.method, path, host);

                if req_len == 0 {
                    serial::println("[HTTP] ERROR: Request too large");
                    return (
                        Box::new(FailedState::new("request too large")),
                        StepResult::Failed("request"),
                    );
                }

                serial::print("[HTTP] Sending ");
                serial::print(self.method);
                serial::print(" ");
                serial::println(path);

                if socket.send_slice(&req_buf[..req_len]).is_err() {
                    serial::println("[HTTP] ERROR: Send failed");
                    return (
                        Box::new(FailedState::new("send failed")),
                        StepResult::Failed("send"),
                    );
                }

                self.phase = HttpPhase::ReceiveHeaders;
                self.last_activity_tsc = tsc;
            },

            HttpPhase::ReceiveHeaders => {
                if !socket.may_recv() {
                    if socket.state() != smoltcp::socket::tcp::State::Established {
                        serial::println("[HTTP] ERROR: Connection closed during headers");
                        return (
                            Box::new(FailedState::new("connection closed")),
                            StepResult::Failed("closed"),
                        );
                    }
                    return (self, StepResult::Continue);
                }

                let space = self.header_buf.len() - self.header_len;
                if space == 0 {
                    serial::println("[HTTP] ERROR: Headers too large");
                    return (
                        Box::new(FailedState::new("headers too large")),
                        StepResult::Failed("headers"),
                    );
                }

                match socket.recv_slice(&mut self.header_buf[self.header_len..]) {
                    Ok(0) => {},
                    Ok(n) => {
                        self.header_len += n;
                        self.last_activity_tsc = tsc;

                        if let Some(end) = find_header_end(&self.header_buf[..self.header_len]) {
                            let header_str =
                                core::str::from_utf8(&self.header_buf[..end]).unwrap_or("");

                            if !header_str.starts_with("HTTP/1.1 200")
                                && !header_str.starts_with("HTTP/1.0 200")
                            {
                                serial::print("[HTTP] ERROR: Bad status: ");
                                if let Some(line_end) = header_str.find('\r') {
                                    serial::println(&header_str[..line_end]);
                                }
                                return (
                                    Box::new(FailedState::new("bad HTTP status")),
                                    StepResult::Failed("status"),
                                );
                            }

                            serial::println("[HTTP] Got 200 OK");

                            self.content_length = parse_content_length(header_str);
                            if let Some(len) = self.content_length {
                                serial::print("[HTTP] Content-Length: ");
                                serial::print_u32((len / 1024 / 1024) as u32);
                                serial::println(" MB");
                                ctx.content_length = Some(len);
                            }

                            self.chunked =
                                contains_ignore_case(header_str, "transfer-encoding: chunked");

                            let body_start = end + 4; // past \r\n\r\n
                            let body_len = self.header_len - body_start;
                            if body_len > 0 {
                                // Body bytes that arrived with the headers.
                                self.bytes_received += body_len as u64;
                                ctx.bytes_downloaded = self.bytes_received;

                                if let (Some(ref mut writer), Some(ref mut blk)) =
                                    (&mut self.disk_writer, &mut ctx.blk_device)
                                {
                                    let written = writer
                                        .write(blk, &self.header_buf[body_start..self.header_len]);
                                    ctx.bytes_written += written as u64;
                                }
                            }

                            self.phase = HttpPhase::ReceiveBody;
                            serial::println("[HTTP] Receiving body...");
                        }
                    },
                    Err(_) => {},
                }
            },

            HttpPhase::ReceiveBody => {
                if !socket.may_recv() {
                    if let Some(expected) = self.content_length {
                        if self.bytes_received >= expected {
                            if let (Some(ref mut writer), Some(ref mut blk)) =
                                (&mut self.disk_writer, &mut ctx.blk_device)
                            {
                                if !writer.flush(blk) {
                                    serial::println("[HTTP] ERROR: Disk flush failed");
                                    return (
                                        Box::new(FailedState::new("disk flush")),
                                        StepResult::Failed("flush"),
                                    );
                                }
                                ctx.bytes_written = writer.bytes_written();
                            }
                            serial::println("[HTTP] Download complete");
                            self.phase = HttpPhase::Complete;
                            ctx.bytes_downloaded = self.bytes_received;
                            return (
                                Box::new(ManifestState::from_context(ctx)),
                                StepResult::Transition,
                            );
                        }
                    }

                    if socket.state() != smoltcp::socket::tcp::State::Established {
                        // No Content-Length: close signals EOF.
                        if self.content_length.is_none() {
                            if let (Some(ref mut writer), Some(ref mut blk)) =
                                (&mut self.disk_writer, &mut ctx.blk_device)
                            {
                                writer.flush(blk);
                                ctx.bytes_written = writer.bytes_written();
                            }
                            serial::println("[HTTP] Download complete (connection closed)");
                            ctx.bytes_downloaded = self.bytes_received;
                            return (
                                Box::new(ManifestState::from_context(ctx)),
                                StepResult::Transition,
                            );
                        }
                        serial::println("[HTTP] ERROR: Premature connection close");
                        return (
                            Box::new(FailedState::new("premature close")),
                            StepResult::Failed("close"),
                        );
                    }
                    return (self, StepResult::Continue);
                }

                let mut buf = [0u8; 4096];
                match socket.recv_slice(&mut buf) {
                    Ok(0) => {},
                    Ok(n) => {
                        self.bytes_received += n as u64;
                        self.last_activity_tsc = tsc;
                        ctx.bytes_downloaded = self.bytes_received;

                        // Log progress on each 1 MB boundary crossed.
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

                        if let (Some(ref mut writer), Some(ref mut blk)) =
                            (&mut self.disk_writer, &mut ctx.blk_device)
                        {
                            let written = writer.write(blk, &buf[..n]);
                            ctx.bytes_written += written as u64;
                        }
                    },
                    Err(_) => {},
                }

                if let Some(expected) = self.content_length {
                    if self.bytes_received >= expected {
                        if let (Some(ref mut writer), Some(ref mut blk)) =
                            (&mut self.disk_writer, &mut ctx.blk_device)
                        {
                            if !writer.flush(blk) {
                                serial::println("[HTTP] ERROR: Final disk flush failed");
                                return (
                                    Box::new(FailedState::new("disk flush")),
                                    StepResult::Failed("flush"),
                                );
                            }
                            ctx.bytes_written = writer.bytes_written();
                        }
                        serial::println("[HTTP] Download complete");
                        ctx.bytes_downloaded = self.bytes_received;
                        return (
                            Box::new(ManifestState::from_context(ctx)),
                            StepResult::Transition,
                        );
                    }
                }
            },

            HttpPhase::Complete => {
                // Completion normally transitions to ManifestState above; this is a fallback.
                return (Box::new(DoneState::new()), StepResult::Transition);
            },
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "HTTP"
    }
}

/// Returns request length, or 0 if the buffer is too small.
fn format_http_request(buf: &mut [u8], method: &str, path: &str, host: &str) -> usize {
    let mut pos = 0;

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

fn find_header_end(data: &[u8]) -> Option<usize> {
    (0..data.len().saturating_sub(3)).find(|&i| &data[i..i + 4] == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<u64> {
    for line in headers.lines() {
        if line.len() >= 15 && line[..15].eq_ignore_ascii_case("content-length:") {
            let value = line[15..].trim();
            return value.parse().ok();
        }
    }
    None
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    for i in 0..=(haystack.len() - needle.len()) {
        if haystack[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            return true;
        }
    }
    false
}
