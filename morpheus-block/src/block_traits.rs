//! Block device trait surface for VirtIO-blk, AHCI, SDHCI, USB-MSD.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    QueueFull,
    DeviceNotReady,
    IoError,
    InvalidSector,
    RequestTooLarge,
    ReadOnly,
    Unsupported,
    Timeout,
    DeviceError,
}

#[derive(Debug, Clone, Copy)]
pub struct BlockCompletion {
    pub request_id: u32,
    /// 0 = success, non-zero = error.
    pub status: u8,
    pub bytes_transferred: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct BlockDeviceInfo {
    pub total_sectors: u64,
    pub sector_size: u32,
    pub max_sectors_per_request: u32,
    pub read_only: bool,
}

/// Non-blocking, fire-and-forget block device interface.
pub trait BlockDriver {
    fn info(&self) -> BlockDeviceInfo;

    fn can_submit(&self) -> bool;

    /// Submit a read. MUST return immediately; buffer must remain valid until completion.
    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError>;

    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError>;

    fn poll_completion(&mut self) -> Option<BlockCompletion>;

    /// Called after one or more submit calls.
    fn notify(&mut self);

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

pub trait BlockDriverInit: Sized {
    type Error: core::fmt::Debug;
    type Config;

    fn supported_vendors() -> &'static [u16];
    fn supported_devices() -> &'static [u16];

    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) && Self::supported_devices().contains(&device)
    }

    /// # Safety
    /// `mmio_base` must be a valid device MMIO address.
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error>;
}
