//! HTTP download state machine.
//!
//! Non-blocking HTTP GET with streaming support.
//!
//! # States
//! Init → Resolving → Connecting → SendingRequest → ReceivingHeaders → ReceivingBody → Done | Failed
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.5

// TODO: Implement HttpDownloadState
//
// pub enum HttpDownloadState {
//     Init { url: Url },
//     Resolving { host: String, port: u16, path: String, start_tsc: u64 },
//     Connecting { ip: Ipv4Addr, port: u16, path: String, tcp: TcpConnState },
//     SendingRequest { socket: SocketHandle, request: Vec<u8>, sent: usize, start_tsc: u64 },
//     ReceivingHeaders { socket: SocketHandle, buffer: Vec<u8>, start_tsc: u64 },
//     ReceivingBody { socket: SocketHandle, headers: HttpHeaders, received: usize, content_length: Option<usize>, start_tsc: u64 },
//     Done { total_bytes: usize },
//     Failed { error: HttpError },
// }
