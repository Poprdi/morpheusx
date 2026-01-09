//! End-to-End ISO Download Orchestration.
//!
//! Integrates HTTP download with disk writing for complete ISO persistence.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                          PersistenceOrchestrator                        │
//! │                                                                         │
//! │  ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐   │
//! │  │  DHCP State     │ ──▶ │  HTTP Download  │ ──▶ │  Disk Writer    │   │
//! │  │  Machine        │     │  State Machine  │     │  State Machine  │   │
//! │  └─────────────────┘     └─────────────────┘     └─────────────────┘   │
//! │           │                      │                       │             │
//! │           ▼                      ▼                       ▼             │
//! │  ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐   │
//! │  │   smoltcp       │     │   smoltcp       │     │   VirtIO-blk    │   │
//! │  │   DHCP          │     │   TCP Socket    │     │   Driver        │   │
//! │  └─────────────────┘     └─────────────────┘     └─────────────────┘   │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```

use crate::driver::block_traits::{BlockDriver, BlockError};
use crate::state::StepResult;
use crate::state::disk_writer::{DiskWriterState, DiskWriterConfig, DiskWriterError, DiskWriterProgress};

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// Persistence orchestrator configuration.
#[derive(Clone)]
pub struct PersistenceConfig<'a> {
    /// URL of the ISO to download.
    pub iso_url: &'a str,
    /// Starting sector on disk for ISO data.
    pub disk_start_sector: u64,
    /// Sector size (default 512).
    pub sector_size: u32,
    /// Expected ISO size (0 = unknown, will use Content-Length).
    pub expected_size: u64,
    /// Whether to verify checksum (future feature).
    pub verify_checksum: bool,
    /// Optional SHA256 checksum to verify.
    pub expected_checksum: Option<[u8; 32]>,
}

impl<'a> Default for PersistenceConfig<'a> {
    fn default() -> Self {
        Self {
            iso_url: "",
            disk_start_sector: 0,
            sector_size: 512,
            expected_size: 0,
            verify_checksum: false,
            expected_checksum: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// Orchestrator error types.
#[derive(Debug, Clone, Copy)]
pub enum OrchestratorError {
    /// Invalid URL.
    InvalidUrl,
    /// DHCP failed.
    DhcpFailed,
    /// DHCP timed out.
    DhcpTimeout,
    /// HTTP error.
    HttpFailed,
    /// Disk write error.
    DiskError(DiskWriterError),
    /// Network device error.
    NetworkError,
    /// Checksum mismatch.
    ChecksumMismatch,
    /// Operation cancelled.
    Cancelled,
    /// Invalid state.
    InvalidState,
}

impl From<DiskWriterError> for OrchestratorError {
    fn from(e: DiskWriterError) -> Self {
        OrchestratorError::DiskError(e)
    }
}

impl From<BlockError> for OrchestratorError {
    fn from(e: BlockError) -> Self {
        OrchestratorError::DiskError(DiskWriterError::BlockError(e))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Orchestrator step result.
#[derive(Debug, Clone, Copy)]
pub enum OrchestratorResult {
    /// Operation in progress.
    Pending,
    /// Download and write completed successfully.
    Done(PersistenceResult),
    /// Operation failed.
    Failed(OrchestratorError),
}

/// Successful persistence result.
#[derive(Debug, Clone, Copy, Default)]
pub struct PersistenceResult {
    /// Total bytes downloaded.
    pub bytes_downloaded: u64,
    /// Total bytes written to disk.
    pub bytes_written: u64,
    /// Starting sector on disk.
    pub start_sector: u64,
    /// Ending sector on disk (exclusive).
    pub end_sector: u64,
    /// Download duration in TSC ticks.
    pub download_ticks: u64,
    /// Write duration in TSC ticks.
    pub write_ticks: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// PROGRESS
// ═══════════════════════════════════════════════════════════════════════════

/// Combined progress tracking.
#[derive(Debug, Clone, Copy, Default)]
pub struct PersistenceProgress {
    /// Current phase.
    pub phase: PersistencePhase,
    /// Bytes downloaded from network.
    pub bytes_downloaded: u64,
    /// Bytes written to disk.
    pub bytes_written: u64,
    /// Total expected bytes (0 = unknown).
    pub total_bytes: u64,
    /// Download percentage (0-100).
    pub download_percent: u8,
    /// Write percentage (0-100).
    pub write_percent: u8,
    /// Pending disk writes.
    pub pending_writes: usize,
}

/// Current persistence phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PersistencePhase {
    #[default]
    Init,
    WaitingForDhcp,
    Connecting,
    Downloading,
    Flushing,
    Verifying,
    Done,
    Failed,
}

// ═══════════════════════════════════════════════════════════════════════════
// STATE MACHINE
// ═══════════════════════════════════════════════════════════════════════════

/// Orchestrator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrchestratorState {
    /// Initial state.
    Init,
    /// Waiting for DHCP.
    WaitingForDhcp,
    /// Connecting to HTTP server.
    Connecting,
    /// Downloading and writing.
    Streaming,
    /// Flushing remaining writes.
    Flushing,
    /// Verifying (optional).
    Verifying,
    /// Complete.
    Done,
    /// Failed.
    Failed,
}

/// Persistence orchestrator.
///
/// Coordinates HTTP download with disk writing for ISO persistence.
/// This is a simplified version that manages disk writing state.
pub struct PersistenceOrchestrator {
    /// Current state.
    state: OrchestratorState,
    /// Disk writer state machine.
    disk_writer: DiskWriterState,
    /// Configuration.
    start_sector: u64,
    expected_size: u64,
    verify_checksum: bool,
    expected_checksum: Option<[u8; 32]>,
    /// Tracking.
    bytes_downloaded: u64,
    /// Timing.
    start_tsc: u64,
    download_start_tsc: u64,
    /// Result.
    result: PersistenceResult,
    /// Error.
    error: Option<OrchestratorError>,
    /// Whether cancelled.
    cancelled: bool,
}

impl PersistenceOrchestrator {
    /// Create new orchestrator with configuration.
    pub fn new(config: PersistenceConfig<'_>) -> Self {
        // Create disk writer config
        let disk_config = DiskWriterConfig {
            start_sector: config.disk_start_sector,
            total_bytes: config.expected_size,
            sector_size: config.sector_size,
        };
        
        Self {
            state: OrchestratorState::Init,
            disk_writer: DiskWriterState::new(disk_config),
            start_sector: config.disk_start_sector,
            expected_size: config.expected_size,
            verify_checksum: config.verify_checksum,
            expected_checksum: config.expected_checksum,
            bytes_downloaded: 0,
            start_tsc: 0,
            download_start_tsc: 0,
            result: PersistenceResult::default(),
            error: None,
            cancelled: false,
        }
    }
    
    /// Get current phase.
    pub fn phase(&self) -> PersistencePhase {
        match self.state {
            OrchestratorState::Init => PersistencePhase::Init,
            OrchestratorState::WaitingForDhcp => PersistencePhase::WaitingForDhcp,
            OrchestratorState::Connecting => PersistencePhase::Connecting,
            OrchestratorState::Streaming => PersistencePhase::Downloading,
            OrchestratorState::Flushing => PersistencePhase::Flushing,
            OrchestratorState::Verifying => PersistencePhase::Verifying,
            OrchestratorState::Done => PersistencePhase::Done,
            OrchestratorState::Failed => PersistencePhase::Failed,
        }
    }
    
    /// Get combined progress.
    pub fn progress(&self) -> PersistenceProgress {
        let disk_progress = self.disk_writer.progress();
        
        let total = if self.expected_size > 0 {
            self.expected_size
        } else {
            0
        };
        
        PersistenceProgress {
            phase: self.phase(),
            bytes_downloaded: self.bytes_downloaded,
            bytes_written: disk_progress.bytes_written,
            total_bytes: total,
            download_percent: if total > 0 {
                ((self.bytes_downloaded * 100) / total).min(100) as u8
            } else {
                0
            },
            write_percent: if total > 0 {
                ((disk_progress.bytes_written * 100) / total).min(100) as u8
            } else {
                0
            },
            pending_writes: disk_progress.pending_writes,
        }
    }
    
    /// Get result (if complete).
    pub fn result(&self) -> Option<PersistenceResult> {
        if self.state == OrchestratorState::Done {
            Some(self.result)
        } else {
            None
        }
    }
    
    /// Get error (if failed).
    pub fn error(&self) -> Option<OrchestratorError> {
        self.error
    }
    
    /// Check if complete.
    pub fn is_complete(&self) -> bool {
        self.state == OrchestratorState::Done
    }
    
    /// Check if failed.
    pub fn is_failed(&self) -> bool {
        self.state == OrchestratorState::Failed
    }
    
    /// Cancel the operation.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }
    
    /// Start the orchestrator.
    ///
    /// Must be called before step().
    pub fn start<B: BlockDriver>(
        &mut self,
        block_driver: &B,
        now_tsc: u64,
    ) -> Result<(), OrchestratorError> {
        if self.state != OrchestratorState::Init {
            return Ok(());
        }
        
        // Start disk writer
        self.disk_writer.start(block_driver)?;
        
        self.start_tsc = now_tsc;
        self.state = OrchestratorState::WaitingForDhcp;
        
        Ok(())
    }
    
    /// Signal that DHCP is complete.
    pub fn dhcp_complete(&mut self, now_tsc: u64) {
        if self.state == OrchestratorState::WaitingForDhcp {
            self.state = OrchestratorState::Connecting;
            self.download_start_tsc = now_tsc;
        }
    }
    
    /// Signal that HTTP connection is established.
    pub fn connected(&mut self) {
        if self.state == OrchestratorState::Connecting {
            self.state = OrchestratorState::Streaming;
        }
    }
    
    /// Set total expected size (from HTTP Content-Length).
    pub fn set_total_bytes(&mut self, total: u64) {
        self.expected_size = total;
        self.disk_writer.set_total_bytes(total);
    }
    
    /// Step the orchestrator (poll completions).
    pub fn step<B: BlockDriver>(
        &mut self,
        block_driver: &mut B,
        now_tsc: u64,
    ) -> OrchestratorResult {
        // Check cancellation
        if self.cancelled {
            self.error = Some(OrchestratorError::Cancelled);
            self.state = OrchestratorState::Failed;
            return OrchestratorResult::Failed(OrchestratorError::Cancelled);
        }
        
        match self.state {
            OrchestratorState::Init => {
                OrchestratorResult::Pending
            }
            
            OrchestratorState::WaitingForDhcp => {
                OrchestratorResult::Pending
            }
            
            OrchestratorState::Connecting => {
                OrchestratorResult::Pending
            }
            
            OrchestratorState::Streaming => {
                // Step disk writer to process completions
                self.disk_writer.step(block_driver);
                OrchestratorResult::Pending
            }
            
            OrchestratorState::Flushing => {
                // Step disk writer
                let result = self.disk_writer.step(block_driver);
                
                match result {
                    StepResult::Done => {
                        if self.verify_checksum {
                            self.state = OrchestratorState::Verifying;
                        } else {
                            self.finalize(now_tsc);
                            self.state = OrchestratorState::Done;
                            return OrchestratorResult::Done(self.result);
                        }
                    }
                    StepResult::Failed => {
                        let err = self.disk_writer.error()
                            .map(OrchestratorError::DiskError)
                            .unwrap_or(OrchestratorError::InvalidState);
                        self.error = Some(err);
                        self.state = OrchestratorState::Failed;
                        return OrchestratorResult::Failed(err);
                    }
                    _ => {}
                }
                
                OrchestratorResult::Pending
            }
            
            OrchestratorState::Verifying => {
                // TODO: Read back data and verify checksum
                // For now, skip verification
                self.finalize(now_tsc);
                self.state = OrchestratorState::Done;
                OrchestratorResult::Done(self.result)
            }
            
            OrchestratorState::Done => {
                OrchestratorResult::Done(self.result)
            }
            
            OrchestratorState::Failed => {
                OrchestratorResult::Failed(self.error.unwrap_or(OrchestratorError::InvalidState))
            }
        }
    }
    
    /// Finalize results.
    fn finalize(&mut self, now_tsc: u64) {
        let disk_progress = self.disk_writer.progress();
        
        self.result = PersistenceResult {
            bytes_downloaded: self.bytes_downloaded,
            bytes_written: disk_progress.bytes_written,
            start_sector: self.start_sector,
            end_sector: self.disk_writer.next_sector(),
            download_ticks: now_tsc.wrapping_sub(self.download_start_tsc),
            write_ticks: now_tsc.wrapping_sub(self.start_tsc),
        };
    }
    
    /// Write a chunk of data to disk.
    ///
    /// Called when HTTP data is received.
    pub fn write_data<B: BlockDriver>(
        &mut self,
        block_driver: &mut B,
        buffer_phys: u64,
        len: usize,
    ) -> Result<(), DiskWriterError> {
        self.bytes_downloaded += len as u64;
        self.disk_writer.write_chunk(block_driver, buffer_phys, len)
    }
    
    /// Signal that HTTP download is complete.
    pub fn http_complete(&mut self) {
        if self.state == OrchestratorState::Streaming {
            self.disk_writer.finish();
            self.state = OrchestratorState::Flushing;
        }
    }
    
    /// Signal HTTP failure.
    pub fn http_failed(&mut self) {
        self.error = Some(OrchestratorError::HttpFailed);
        self.state = OrchestratorState::Failed;
    }
    
    /// Check if writer can accept more data (backpressure).
    pub fn can_write(&self) -> bool {
        self.disk_writer.can_write()
    }
    
    /// Get disk writer state.
    pub fn disk_writer(&self) -> &DiskWriterState {
        &self.disk_writer
    }
    
    /// Get mutable disk writer.
    pub fn disk_writer_mut(&mut self) -> &mut DiskWriterState {
        &mut self.disk_writer
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_config_default() {
        let config = PersistenceConfig::default();
        assert_eq!(config.disk_start_sector, 0);
        assert_eq!(config.sector_size, 512);
        assert!(!config.verify_checksum);
    }
    
    #[test]
    fn test_progress_default() {
        let progress = PersistenceProgress::default();
        assert_eq!(progress.phase, PersistencePhase::Init);
        assert_eq!(progress.bytes_downloaded, 0);
        assert_eq!(progress.bytes_written, 0);
    }
}
