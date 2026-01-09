//! Block device trait definitions.
//!
//! Defines the interface for block storage devices (VirtIO-blk, etc.).

/// Block I/O error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    /// Request queue is full.
    QueueFull,
    /// Device not ready.
    DeviceNotReady,
    /// I/O error from device.
    IoError,
    /// Invalid sector number.
    InvalidSector,
    /// Request too large.
    RequestTooLarge,
    /// Device is read-only.
    ReadOnly,
    /// Unsupported operation.
    Unsupported,
}

/// Result of a completed block I/O operation.
#[derive(Debug, Clone, Copy)]
pub struct BlockCompletion {
    /// Request ID (for matching with submit)
    pub request_id: u32,
    /// Status: 0 = success, non-zero = error
    pub status: u8,
    /// Bytes transferred
    pub bytes_transferred: u32,
}

/// Block device information.
#[derive(Debug, Clone, Copy)]
pub struct BlockDeviceInfo {
    /// Total capacity in sectors
    pub total_sectors: u64,
    /// Logical block (sector) size in bytes
    pub sector_size: u32,
    /// Maximum sectors per request
    pub max_sectors_per_request: u32,
    /// Whether device is read-only
    pub read_only: bool,
}

/// Core block device interface.
///
/// All block drivers must implement this trait.
/// Operations are non-blocking with fire-and-forget semantics.
pub trait BlockDriver {
    /// Get device information.
    fn info(&self) -> BlockDeviceInfo;
    
    /// Check if device can accept a new request.
    fn can_submit(&self) -> bool;
    
    /// Submit a read request.
    ///
    /// # Arguments
    /// - `sector`: Starting sector number
    /// - `buffer_phys`: Physical address of destination buffer
    /// - `num_sectors`: Number of sectors to read
    /// - `request_id`: Caller-provided ID for tracking
    ///
    /// # Returns
    /// - `Ok(())`: Request submitted
    /// - `Err(BlockError)`: Submit failed
    ///
    /// # Contract
    /// - MUST return immediately (fire-and-forget)
    /// - Buffer must remain valid until completion
    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError>;
    
    /// Submit a write request.
    ///
    /// # Arguments
    /// - `sector`: Starting sector number
    /// - `buffer_phys`: Physical address of source buffer
    /// - `num_sectors`: Number of sectors to write
    /// - `request_id`: Caller-provided ID for tracking
    ///
    /// # Returns
    /// - `Ok(())`: Request submitted
    /// - `Err(BlockError)`: Submit failed
    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError>;
    
    /// Poll for completed requests.
    ///
    /// # Returns
    /// - `Some(completion)`: A request completed
    /// - `None`: No completions available
    fn poll_completion(&mut self) -> Option<BlockCompletion>;
    
    /// Notify device that requests are pending.
    ///
    /// Called after one or more submit calls.
    fn notify(&mut self);
    
    /// Flush any pending writes (if supported).
    fn flush(&mut self) -> Result<(), BlockError> {
        // Default: no-op for devices without flush support
        Ok(())
    }
}

/// Block driver initialization trait.
pub trait BlockDriverInit: Sized {
    /// Error type for initialization failures.
    type Error: core::fmt::Debug;
    
    /// Configuration type.
    type Config;
    
    /// PCI vendor IDs this driver supports.
    fn supported_vendors() -> &'static [u16];
    
    /// PCI device IDs this driver supports.
    fn supported_devices() -> &'static [u16];
    
    /// Check if driver supports a PCI device.
    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) &&
        Self::supported_devices().contains(&device)
    }
    
    /// Create driver from MMIO base and configuration.
    ///
    /// # Safety
    /// - `mmio_base` must be valid device MMIO address
    /// - Configuration must be valid
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error>;
}
