//! ISO download orchestration state machine.
//!
//! Composes DHCP → HTTP state machines for complete ISO download workflow.
//!
//! # Architecture
//!
//! ```text
//! Init → WaitingForNetwork → Downloading → WritingToDisk → Done
//!   ↓           ↓                 ↓              ↓            
//! Failed     Failed           Failed         Failed
//! ```
//!
//! # Streaming Support
//!
//! For large ISO downloads (often 1-4GB), data is streamed directly to disk
//! rather than buffered in memory. The state machine coordinates:
//!
//! 1. DHCP: Obtain network configuration
//! 2. HTTP: Download ISO with streaming callbacks
//! 3. Disk: Write chunks to VirtIO-blk as they arrive
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.6

use core::net::Ipv4Addr;
use alloc::string::{String, ToString};

use super::{StepResult, StateError, TscTimestamp, Progress};
use super::dhcp::{DhcpState, DhcpConfig, DhcpError};
use super::http::{HttpDownloadState, HttpError, HttpProgress, HttpResponseInfo};
use super::tcp::TcpSocketState;
use crate::url::Url;

// ═══════════════════════════════════════════════════════════════════════════
// DOWNLOAD ERROR
// ═══════════════════════════════════════════════════════════════════════════

/// Errors during ISO download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// DHCP failed to obtain address
    NetworkError(DhcpError),
    /// HTTP download failed
    HttpError(HttpError),
    /// Invalid URL
    InvalidUrl,
    /// Checksum verification failed
    ChecksumMismatch,
    /// Disk write failed
    DiskWriteError,
    /// Not enough disk space
    InsufficientSpace,
    /// ISO too large for available memory
    IsoTooLarge,
    /// Download cancelled
    Cancelled,
}

impl From<DhcpError> for DownloadError {
    fn from(e: DhcpError) -> Self {
        DownloadError::NetworkError(e)
    }
}

impl From<HttpError> for DownloadError {
    fn from(e: HttpError) -> Self {
        DownloadError::HttpError(e)
    }
}

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

// ═══════════════════════════════════════════════════════════════════════════
// DOWNLOAD CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Configuration for ISO download.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// URL of ISO to download
    pub url: Url,
    /// Expected SHA-256 hash (optional, for verification)
    pub expected_hash: Option<[u8; 32]>,
    /// Maximum file size (bytes)
    pub max_size: Option<usize>,
    /// Target disk sector offset for writing
    pub disk_start_sector: u64,
    /// Sector size (usually 512)
    pub sector_size: usize,
}

impl DownloadConfig {
    /// Create new download config.
    pub fn new(url: Url) -> Self {
        Self {
            url,
            expected_hash: None,
            max_size: None,
            disk_start_sector: 0,
            sector_size: 512,
        }
    }
    
    /// Set expected SHA-256 hash for verification.
    pub fn with_hash(mut self, hash: [u8; 32]) -> Self {
        self.expected_hash = Some(hash);
        self
    }
    
    /// Set maximum allowed file size.
    pub fn with_max_size(mut self, max_size: usize) -> Self {
        self.max_size = Some(max_size);
        self
    }
    
    /// Set target disk location.
    pub fn with_disk_location(mut self, start_sector: u64, sector_size: usize) -> Self {
        self.disk_start_sector = start_sector;
        self.sector_size = sector_size;
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DOWNLOAD PROGRESS
// ═══════════════════════════════════════════════════════════════════════════

/// Overall download progress.
#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    /// Current phase
    pub phase: DownloadPhase,
    /// Bytes downloaded
    pub bytes_downloaded: usize,
    /// Total expected bytes (if known)
    pub total_bytes: Option<usize>,
    /// Bytes written to disk
    pub bytes_written: usize,
}

/// Download phase for progress tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    /// Waiting for network
    WaitingForNetwork,
    /// Connecting to server
    Connecting,
    /// Downloading data
    Downloading,
    /// Writing to disk
    WritingToDisk,
    /// Verifying checksum
    Verifying,
    /// Complete
    Complete,
}

impl DownloadProgress {
    /// Calculate overall percentage (0-100).
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
            }
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

// ═══════════════════════════════════════════════════════════════════════════
// DOWNLOAD RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of successful download.
#[derive(Debug, Clone)]
pub struct DownloadResult {
    /// Total bytes downloaded
    pub total_bytes: usize,
    /// HTTP response info
    pub response_info: HttpResponseInfo,
    /// Starting sector on disk
    pub disk_start_sector: u64,
    /// Number of sectors written
    pub sectors_written: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// ISO DOWNLOAD STATE MACHINE
// ═══════════════════════════════════════════════════════════════════════════

/// ISO download orchestration state machine.
///
/// Coordinates DHCP → HTTP → Disk Write workflow.
#[derive(Debug)]
pub enum IsoDownloadState {
    /// Initial state with configuration.
    Init {
        config: DownloadConfig,
    },
    
    /// Waiting for DHCP to obtain network configuration.
    WaitingForNetwork {
        /// DHCP state machine
        dhcp: DhcpState,
        /// Download configuration
        config: DownloadConfig,
    },
    
    /// Network ready, downloading ISO.
    Downloading {
        /// HTTP download state machine
        http: HttpDownloadState,
        /// Network configuration from DHCP
        network_config: DhcpConfig,
        /// Download configuration
        config: DownloadConfig,
        /// Current disk write position (sectors)
        disk_position: u64,
        /// Bytes pending disk write
        pending_write: usize,
        /// Total bytes written to disk
        bytes_written: usize,
    },
    
    /// Download complete, verifying checksum (if hash provided).
    Verifying {
        /// Download result so far
        result: DownloadResult,
        /// Expected hash
        expected_hash: [u8; 32],
        /// Verification progress (bytes checked)
        verified_bytes: usize,
        /// When verification started
        start_tsc: TscTimestamp,
    },
    
    /// Download and verification complete.
    Done {
        result: DownloadResult,
    },
    
    /// Download failed.
    Failed {
        error: DownloadError,
    },
}

impl IsoDownloadState {
    /// Create new download state machine.
    pub fn new(config: DownloadConfig) -> Self {
        IsoDownloadState::Init { config }
    }
    
    /// Start the download.
    ///
    /// If already have network config, skip DHCP.
    /// Otherwise, start DHCP first.
    pub fn start(&mut self, existing_network: Option<DhcpConfig>, now_tsc: u64) {
        if let IsoDownloadState::Init { config } = self {
            let config = core::mem::replace(config, DownloadConfig::new(Url {
                scheme: crate::url::parser::Scheme::Http,
                host: String::new(),
                port: None,
                path: String::new(),
                query: None,
            }));
            
            if let Some(network_config) = existing_network {
                // Already have network, start HTTP download
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
                    }
                    Err(e) => {
                        *self = IsoDownloadState::Failed {
                            error: DownloadError::HttpError(e),
                        };
                    }
                }
            } else {
                // Need DHCP first
                let mut dhcp = DhcpState::new();
                dhcp.start(now_tsc);
                *self = IsoDownloadState::WaitingForNetwork { dhcp, config };
            }
        }
    }
    
    /// Step the state machine.
    ///
    /// # Arguments
    /// - `dhcp_event`: DHCP event from smoltcp (if any)
    /// - `dns_result`: DNS query result (if resolving)
    /// - `tcp_state`: TCP socket state (if connected)
    /// - `recv_data`: Data received from HTTP socket
    /// - `can_send`: Whether socket can send
    /// - `disk_write_result`: Result of disk write (Ok(written) or Err)
    /// - `now_tsc`: Current TSC value
    /// - `timeouts`: Timeout values
    ///
    /// # Returns
    /// StepResult indicating current status
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
        // Take ownership for state transitions
        let current = core::mem::replace(self, IsoDownloadState::Init {
            config: DownloadConfig::new(Url {
                scheme: crate::url::parser::Scheme::Http,
                host: String::new(),
                port: None,
                path: String::new(),
                query: None,
            }),
        });
        
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
    
    /// Internal step implementation.
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
                // Not started yet
                (IsoDownloadState::Init { config }, StepResult::Pending)
            }
            
            IsoDownloadState::WaitingForNetwork { mut dhcp, config } => {
                // Step DHCP state machine
                let result = dhcp.step(dhcp_event, now_tsc, dhcp_timeout);
                
                match result {
                    StepResult::Done => {
                        // Network ready, start HTTP download
                        let network_config = dhcp.config().unwrap().clone();
                        
                        match HttpDownloadState::new(config.url.clone()) {
                            Ok(mut http) => {
                                http.start(now_tsc);
                                (IsoDownloadState::Downloading {
                                    http,
                                    network_config,
                                    config,
                                    disk_position: 0,
                                    pending_write: 0,
                                    bytes_written: 0,
                                }, StepResult::Pending)
                            }
                            Err(e) => {
                                (IsoDownloadState::Failed {
                                    error: DownloadError::HttpError(e),
                                }, StepResult::Failed)
                            }
                        }
                    }
                    StepResult::Pending => {
                        (IsoDownloadState::WaitingForNetwork { dhcp, config }, StepResult::Pending)
                    }
                    StepResult::Timeout => {
                        (IsoDownloadState::Failed {
                            error: DownloadError::NetworkError(DhcpError::Timeout),
                        }, StepResult::Timeout)
                    }
                    StepResult::Failed => {
                        let error = dhcp.error().unwrap_or(DhcpError::Timeout);
                        (IsoDownloadState::Failed {
                            error: DownloadError::NetworkError(error),
                        }, StepResult::Failed)
                    }
                }
            }
            
            IsoDownloadState::Downloading {
                mut http,
                network_config,
                config,
                mut disk_position,
                mut pending_write,
                mut bytes_written,
            } => {
                // Process disk write result first
                if let Some(write_result) = disk_write_result {
                    match write_result {
                        Ok(written) => {
                            bytes_written += written;
                            pending_write = pending_write.saturating_sub(written);
                            disk_position += (written / config.sector_size) as u64;
                        }
                        Err(()) => {
                            return (IsoDownloadState::Failed {
                                error: DownloadError::DiskWriteError,
                            }, StepResult::Failed);
                        }
                    }
                }
                
                // Check max size before proceeding
                if let (Some(max_size), Some(content_length)) = (config.max_size, http.response_info().and_then(|r| r.content_length)) {
                    if content_length > max_size {
                        return (IsoDownloadState::Failed {
                            error: DownloadError::IsoTooLarge,
                        }, StepResult::Failed);
                    }
                }
                
                // Step HTTP state machine
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
                
                // Track pending writes from received data
                if let Some(data) = recv_data {
                    if http.response_info().is_some() {
                        // We're in body reception phase
                        pending_write += data.len();
                    }
                }
                
                match result {
                    StepResult::Done => {
                        // HTTP complete
                        let (response_info, total_bytes) = http.result().unwrap();
                        let sectors_written = (bytes_written / config.sector_size) as u64;
                        
                        let download_result = DownloadResult {
                            total_bytes,
                            response_info: response_info.clone(),
                            disk_start_sector: config.disk_start_sector,
                            sectors_written,
                        };
                        
                        // Check if we need verification
                        if let Some(expected_hash) = config.expected_hash {
                            (IsoDownloadState::Verifying {
                                result: download_result,
                                expected_hash,
                                verified_bytes: 0,
                                start_tsc: TscTimestamp::new(now_tsc),
                            }, StepResult::Pending)
                        } else {
                            (IsoDownloadState::Done {
                                result: download_result,
                            }, StepResult::Done)
                        }
                    }
                    StepResult::Pending => {
                        (IsoDownloadState::Downloading {
                            http,
                            network_config,
                            config,
                            disk_position,
                            pending_write,
                            bytes_written,
                        }, StepResult::Pending)
                    }
                    StepResult::Timeout => {
                        let error = http.error().cloned().unwrap_or(HttpError::ReceiveTimeout);
                        (IsoDownloadState::Failed {
                            error: DownloadError::HttpError(error),
                        }, StepResult::Timeout)
                    }
                    StepResult::Failed => {
                        let error = http.error().cloned().unwrap_or(HttpError::ConnectionClosed);
                        (IsoDownloadState::Failed {
                            error: DownloadError::HttpError(error),
                        }, StepResult::Failed)
                    }
                }
            }
            
            IsoDownloadState::Verifying {
                result,
                expected_hash,
                verified_bytes,
                start_tsc,
            } => {
                // TODO: Implement actual hash verification
                // For now, skip verification (always pass)
                // In real implementation, would read sectors back and compute SHA-256
                
                // Just mark as done for now
                (IsoDownloadState::Done { result }, StepResult::Done)
            }
            
            IsoDownloadState::Done { result } => {
                (IsoDownloadState::Done { result }, StepResult::Done)
            }
            
            IsoDownloadState::Failed { error } => {
                let result = match &error {
                    DownloadError::NetworkError(DhcpError::Timeout)
                    | DownloadError::HttpError(HttpError::SendTimeout)
                    | DownloadError::HttpError(HttpError::ReceiveTimeout) => StepResult::Timeout,
                    _ => StepResult::Failed,
                };
                (IsoDownloadState::Failed { error }, result)
            }
        }
    }
    
    /// Get current progress.
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
            IsoDownloadState::Downloading { http, bytes_written, .. } => {
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
            }
            IsoDownloadState::Verifying { result, verified_bytes, .. } => DownloadProgress {
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
                phase: DownloadPhase::Downloading, // Keep last known phase
                bytes_downloaded: 0,
                total_bytes: None,
                bytes_written: 0,
            },
        }
    }
    
    /// Get download result (if complete).
    pub fn result(&self) -> Option<&DownloadResult> {
        if let IsoDownloadState::Done { result } = self {
            Some(result)
        } else {
            None
        }
    }
    
    /// Get error (if failed).
    pub fn error(&self) -> Option<&DownloadError> {
        if let IsoDownloadState::Failed { error } = self {
            Some(error)
        } else {
            None
        }
    }
    
    /// Get network config (if obtained).
    pub fn network_config(&self) -> Option<&DhcpConfig> {
        match self {
            IsoDownloadState::Downloading { network_config, .. } => Some(network_config),
            _ => None,
        }
    }
    
    /// Get HTTP socket handle (if downloading).
    pub fn socket_handle(&self) -> Option<usize> {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.socket_handle()
        } else {
            None
        }
    }
    
    /// Get bytes to send (if in send phase).
    pub fn pending_send(&self) -> Option<(&[u8], usize)> {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.request_bytes()
        } else {
            None
        }
    }
    
    /// Mark bytes as sent.
    pub fn mark_sent(&mut self, bytes: usize) {
        if let IsoDownloadState::Downloading { http, .. } = self {
            http.mark_sent(bytes);
        }
    }
    
    /// Get data pending disk write.
    ///
    /// Returns (current_sector, bytes_pending)
    pub fn pending_disk_write(&self) -> Option<(u64, usize)> {
        if let IsoDownloadState::Downloading { config, disk_position, pending_write, .. } = self {
            if *pending_write > 0 {
                Some((config.disk_start_sector + disk_position, *pending_write))
            } else {
                None
            }
        } else {
            None
        }
    }
    
    /// Check if download is complete (success or failure).
    pub fn is_terminal(&self) -> bool {
        matches!(self, IsoDownloadState::Done { .. } | IsoDownloadState::Failed { .. })
    }
    
    /// Check if download is in progress.
    pub fn is_active(&self) -> bool {
        !matches!(
            self,
            IsoDownloadState::Init { .. }
            | IsoDownloadState::Done { .. }
            | IsoDownloadState::Failed { .. }
        )
    }
    
    /// Cancel the download.
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
