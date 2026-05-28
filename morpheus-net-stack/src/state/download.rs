//! ISO download orchestrator composing DHCP -> HTTP -> disk write, streaming
//! multi-GB ISOs straight to VirtIO-blk rather than buffering in memory.
//!
//! Init -> WaitingForNetwork -> Downloading -> Done, with Failed reachable
//! from each active state.

use alloc::string::{String, ToString};
use core::net::Ipv4Addr;

use super::dhcp::{DhcpConfig, DhcpError, DhcpState};
use super::http::{HttpDownloadState, HttpError, HttpProgress, HttpResponseInfo};
use super::tcp::TcpSocketState;
use super::{Progress, StateError, StepResult, TscTimestamp};
use crate::url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    NetworkError(DhcpError),
    HttpError(HttpError),
    InvalidUrl,
    ChecksumMismatch,
    DiskWriteError,
    InsufficientSpace,
    IsoTooLarge,
    Cancelled,
}

crate::impl_from!(DhcpError => DownloadError : NetworkError);
crate::impl_from!(HttpError => DownloadError : HttpError);

impl From<DownloadError> for StateError {
    fn from(e: DownloadError) -> Self {
        match e {
            DownloadError::NetworkError(_) => StateError::InterfaceError,
            DownloadError::HttpError(ref he) => StateError::from(he.clone()),
            DownloadError::InvalidUrl => StateError::InvalidResponse,
            DownloadError::ChecksumMismatch => StateError::InvalidResponse,
            DownloadError::DiskWriteError => StateError::Internal,
            DownloadError::InsufficientSpace => StateError::Internal,
            DownloadError::IsoTooLarge => StateError::BufferTooSmall,
            DownloadError::Cancelled => StateError::Internal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub url: Url,
    /// Optional SHA-256 for post-download verification.
    pub expected_hash: Option<[u8; 32]>,
    pub max_size: Option<usize>,
    pub disk_start_sector: u64,
    pub sector_size: usize,
}

impl DownloadConfig {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            expected_hash: None,
            max_size: None,
            disk_start_sector: 0,
            sector_size: 512,
        }
    }

    pub fn with_hash(mut self, hash: [u8; 32]) -> Self {
        self.expected_hash = Some(hash);
        self
    }

    pub fn with_max_size(mut self, max_size: usize) -> Self {
        self.max_size = Some(max_size);
        self
    }

    pub fn with_disk_location(mut self, start_sector: u64, sector_size: usize) -> Self {
        self.disk_start_sector = start_sector;
        self.sector_size = sector_size;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub phase: DownloadPhase,
    pub bytes_downloaded: usize,
    pub total_bytes: Option<usize>,
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    WaitingForNetwork,
    Connecting,
    Downloading,
    WritingToDisk,
    Verifying,
    Complete,
}

impl DownloadProgress {
    /// 0-100, weighting connect at 5% and download/write at 5-95%.
    pub fn percent(&self) -> Option<u8> {
        match self.phase {
            DownloadPhase::WaitingForNetwork => Some(0),
            DownloadPhase::Connecting => Some(5),
            DownloadPhase::Downloading | DownloadPhase::WritingToDisk => {
                self.total_bytes.map(|total| {
                    if total == 0 {
                        100
                    } else {
                        let pct = (self.bytes_downloaded as u64 * 90) / total as u64;
                        (5 + pct).min(95) as u8
                    }
                })
            },
            DownloadPhase::Verifying => Some(95),
            DownloadPhase::Complete => Some(100),
        }
    }
}

impl From<DownloadProgress> for Progress {
    fn from(p: DownloadProgress) -> Self {
        Progress {
            bytes_done: p.bytes_downloaded as u64,
            bytes_total: p.total_bytes.unwrap_or(0) as u64,
            start_tsc: 0,
            last_update_tsc: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub total_bytes: usize,
    pub response_info: HttpResponseInfo,
    pub disk_start_sector: u64,
    pub sectors_written: u64,
}

#[derive(Debug)]
pub(crate) enum IsoDownloadState {
    Init {
        config: DownloadConfig,
    },
    WaitingForNetwork {
        dhcp: DhcpState,
        config: DownloadConfig,
    },
    Downloading {
        http: HttpDownloadState,
        network_config: DhcpConfig,
        config: DownloadConfig,
        disk_position: u64,
        pending_write: usize,
        bytes_written: usize,
    },
    Verifying {
        result: DownloadResult,
        expected_hash: [u8; 32],
        verified_bytes: usize,
        start_tsc: TscTimestamp,
    },
    Done {
        result: DownloadResult,
    },
    Failed {
        error: DownloadError,
    },
}

impl IsoDownloadState {
    pub fn new(config: DownloadConfig) -> Self {
        IsoDownloadState::Init { config }
    }

    /// Skip DHCP if `existing_network` is provided, else run DHCP first.
    pub fn start(&mut self, existing_network: Option<DhcpConfig>, now_tsc: u64) {
        if let IsoDownloadState::Init { config } = self {
            let config = core::mem::replace(
                config,
                DownloadConfig::new(Url {
                    scheme: crate::url::parser::Scheme::Http,
                    host: String::new(),
                    port: None,
                    path: String::new(),
                    query: None,
                }),
            );

            if let Some(network_config) = existing_network {
                match HttpDownloadState::new(config.url.clone()) {
                    Ok(mut http) => {
                        http.start(now_tsc);
                        *self = IsoDownloadState::Downloading {
                            http,
                            network_config,
                            config,
                            disk_position: 0,
                            pending_write: 0,
                            bytes_written: 0,
                        };
                    },
                    Err(e) => {
                        *self = IsoDownloadState::Failed {
                            error: DownloadError::HttpError(e),
                        };
                    },
                }
            } else {
                let mut dhcp = DhcpState::new();
                dhcp.start(now_tsc);
                *self = IsoDownloadState::WaitingForNetwork { dhcp, config };
            }
        }
    }

    pub fn step(
        &mut self,
        dhcp_event: Option<Result<DhcpConfig, ()>>,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        can_send: bool,
        disk_write_result: Option<Result<usize, ()>>,
        now_tsc: u64,
        dhcp_timeout: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> StepResult {
        let current = core::mem::replace(
            self,
            IsoDownloadState::Init {
                config: DownloadConfig::new(Url {
                    scheme: crate::url::parser::Scheme::Http,
                    host: String::new(),
                    port: None,
                    path: String::new(),
                    query: None,
                }),
            },
        );

        let (new_state, result) = self.step_inner(
            current,
            dhcp_event,
            dns_result,
            tcp_state,
            recv_data,
            can_send,
            disk_write_result,
            now_tsc,
            dhcp_timeout,
            dns_timeout,
            tcp_timeout,
            http_send_timeout,
            http_recv_timeout,
        );

        *self = new_state;
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn step_inner(
        &self,
        current: IsoDownloadState,
        dhcp_event: Option<Result<DhcpConfig, ()>>,
        dns_result: Result<Option<Ipv4Addr>, ()>,
        tcp_state: TcpSocketState,
        recv_data: Option<&[u8]>,
        can_send: bool,
        disk_write_result: Option<Result<usize, ()>>,
        now_tsc: u64,
        dhcp_timeout: u64,
        dns_timeout: u64,
        tcp_timeout: u64,
        http_send_timeout: u64,
        http_recv_timeout: u64,
    ) -> (IsoDownloadState, StepResult) {
        match current {
            IsoDownloadState::Init { config } => {
                (IsoDownloadState::Init { config }, StepResult::Pending)
            },

            IsoDownloadState::WaitingForNetwork { mut dhcp, config } => {
                let dhcp_config = dhcp_event.and_then(|r| r.ok());
                let result = dhcp.step(dhcp_config, now_tsc, dhcp_timeout);

                match result {
                    StepResult::Done => {
                        let network_config = *dhcp.config().unwrap();

                        match HttpDownloadState::new(config.url.clone()) {
                            Ok(mut http) => {
                                http.start(now_tsc);
                                (
                                    IsoDownloadState::Downloading {
                                        http,
                                        network_config,
                                        config,
                                        disk_position: 0,
                                        pending_write: 0,
                                        bytes_written: 0,
                                    },
                                    StepResult::Pending,
                                )
                            },
                            Err(e) => (
                                IsoDownloadState::Failed {
                                    error: DownloadError::HttpError(e),
                                },
                                StepResult::Failed,
                            ),
                        }
                    },
                    StepResult::Pending => (
                        IsoDownloadState::WaitingForNetwork { dhcp, config },
                        StepResult::Pending,
                    ),
                    StepResult::Timeout => (
                        IsoDownloadState::Failed {
                            error: DownloadError::NetworkError(DhcpError::Timeout),
                        },
                        StepResult::Timeout,
                    ),
                    StepResult::Failed => {
                        let error = dhcp.error().unwrap_or(DhcpError::Timeout);
                        (
                            IsoDownloadState::Failed {
                                error: DownloadError::NetworkError(error),
                            },
                            StepResult::Failed,
                        )
                    },
                }
            },

            IsoDownloadState::Downloading {
                mut http,
                network_config,
                config,
                mut disk_position,
                mut pending_write,
                mut bytes_written,
            } => {
                if let Some(write_result) = disk_write_result {
                    match write_result {
                        Ok(written) => {
                            bytes_written += written;
                            pending_write = pending_write.saturating_sub(written);
                            disk_position += (written / config.sector_size) as u64;
                        },
                        Err(()) => {
                            return (
                                IsoDownloadState::Failed {
                                    error: DownloadError::DiskWriteError,
                                },
                                StepResult::Failed,
                            );
                        },
                    }
                }

                if let (Some(max_size), Some(content_length)) = (
                    config.max_size,
                    http.response_info().and_then(|r| r.content_length),
                ) {
                    if content_length > max_size {
                        return (
                            IsoDownloadState::Failed {
                                error: DownloadError::IsoTooLarge,
                            },
                            StepResult::Failed,
                        );
                    }
                }

                let result = http.step(
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

                // Body bytes (response_info present) become pending disk writes.
                if let Some(data) = recv_data {
                    if http.response_info().is_some() {
                        pending_write += data.len();
                    }
                }

                match result {
                    StepResult::Done => {
                        let (response_info, total_bytes) = http.result().unwrap();
                        let sectors_written = (bytes_written / config.sector_size) as u64;

                        let download_result = DownloadResult {
                            total_bytes,
                            response_info: response_info.clone(),
                            disk_start_sector: config.disk_start_sector,
                            sectors_written,
                        };

                        if let Some(expected_hash) = config.expected_hash {
                            (
                                IsoDownloadState::Verifying {
                                    result: download_result,
                                    expected_hash,
                                    verified_bytes: 0,
                                    start_tsc: TscTimestamp::new(now_tsc),
                                },
                                StepResult::Pending,
                            )
                        } else {
                            (
                                IsoDownloadState::Done {
                                    result: download_result,
                                },
                                StepResult::Done,
                            )
                        }
                    },
                    StepResult::Pending => (
                        IsoDownloadState::Downloading {
                            http,
                            network_config,
                            config,
                            disk_position,
                            pending_write,
                            bytes_written,
                        },
                        StepResult::Pending,
                    ),
                    StepResult::Timeout => {
                        let error = http.error().cloned().unwrap_or(HttpError::ReceiveTimeout);
                        (
                            IsoDownloadState::Failed {
                                error: DownloadError::HttpError(error),
                            },
                            StepResult::Timeout,
                        )
                    },
                    StepResult::Failed => {
                        let error = http.error().cloned().unwrap_or(HttpError::ConnectionClosed);
                        (
                            IsoDownloadState::Failed {
                                error: DownloadError::HttpError(error),
                            },
                            StepResult::Failed,
                        )
                    },
                }
            },

            IsoDownloadState::Verifying {
                result,
                expected_hash: _,
                verified_bytes: _,
                start_tsc: _,
            } => {
                // TODO: read sectors back and compare SHA-256; currently a no-op pass.
                (IsoDownloadState::Done { result }, StepResult::Done)
            },

            IsoDownloadState::Done { result } => {
                (IsoDownloadState::Done { result }, StepResult::Done)
            },

            IsoDownloadState::Failed { error } => {
                let result = match &error {
                    DownloadError::NetworkError(DhcpError::Timeout)
                    | DownloadError::HttpError(HttpError::SendTimeout)
                    | DownloadError::HttpError(HttpError::ReceiveTimeout) => StepResult::Timeout,
                    _ => StepResult::Failed,
                };
                (IsoDownloadState::Failed { error }, result)
            },
        }
    }

    pub fn progress(&self) -> DownloadProgress {
        match self {
            IsoDownloadState::Init { .. } => DownloadProgress {
                phase: DownloadPhase::WaitingForNetwork,
                bytes_downloaded: 0,
                total_bytes: None,
                bytes_written: 0,
            },
            IsoDownloadState::WaitingForNetwork { .. } => DownloadProgress {
                phase: DownloadPhase::WaitingForNetwork,
                bytes_downloaded: 0,
                total_bytes: None,
                bytes_written: 0,
            },
            IsoDownloadState::Downloading {
                http,
                bytes_written,
                ..
            } => {
                let http_progress = http.progress();
                DownloadProgress {
                    phase: if http_progress.is_some() {
                        DownloadPhase::Downloading
                    } else {
                        DownloadPhase::Connecting
                    },
                    bytes_downloaded: http_progress.map(|p| p.received).unwrap_or(0),
                    total_bytes: http_progress.and_then(|p| p.total),
                    bytes_written: *bytes_written,
                }
            },
            IsoDownloadState::Verifying { result, .. } => DownloadProgress {
                phase: DownloadPhase::Verifying,
                bytes_downloaded: result.total_bytes,
                total_bytes: Some(result.total_bytes),
                bytes_written: result.total_bytes,
            },
            IsoDownloadState::Done { result } => DownloadProgress {
                phase: DownloadPhase::Complete,
                bytes_downloaded: result.total_bytes,
                total_bytes: Some(result.total_bytes),
                bytes_written: result.total_bytes,
            },
            IsoDownloadState::Failed { .. } => DownloadProgress {
                phase: DownloadPhase::Downloading,
                bytes_downloaded: 0,
                total_bytes: None,
                bytes_written: 0,
            },
        }
    }

    pub fn result(&self) -> Option<&DownloadResult> {
        if let IsoDownloadState::Done { result } = self {
            Some(result)
        } else {
            None
        }
    }

    pub fn error(&self) -> Option<&DownloadError> {
        if let IsoDownloadState::Failed { error } = self {
            Some(error)
        } else {
            None
        }
    }

    pub fn network_config(&self) -> Option<&DhcpConfig> {
        match self {
            IsoDownloadState::Downloading { network_config, .. } => Some(network_config),
            _ => None,
        }
    }

    pub fn socket_handle(&self) -> Option<usize> {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.socket_handle()
        } else {
            None
        }
    }

    pub fn pending_send(&self) -> Option<(&[u8], usize)> {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.request_bytes()
        } else {
            None
        }
    }

    pub fn mark_sent(&mut self, bytes: usize) {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.mark_sent(bytes);
        }
    }

    /// `(current_sector, bytes_pending)` if any data awaits writing.
    pub fn pending_disk_write(&self) -> Option<(u64, usize)> {
        if let IsoDownloadState::Downloading {
            config,
            disk_position,
            pending_write,
            ..
        } = self
        {
            if *pending_write > 0 {
                Some((config.disk_start_sector + disk_position, *pending_write))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Done or failed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IsoDownloadState::Done { .. } | IsoDownloadState::Failed { .. }
        )
    }

    pub fn is_active(&self) -> bool {
        !matches!(
            self,
            IsoDownloadState::Init { .. }
                | IsoDownloadState::Done { .. }
                | IsoDownloadState::Failed { .. }
        )
    }

    pub fn cancel(&mut self) {
        *self = IsoDownloadState::Failed {
            error: DownloadError::Cancelled,
        };
    }
}

impl Default for IsoDownloadState {
    fn default() -> Self {
        IsoDownloadState::Init {
            config: DownloadConfig::new(Url {
                scheme: crate::url::parser::Scheme::Http,
                host: String::new(),
                port: None,
                path: String::new(),
                query: None,
            }),
        }
    }
}
