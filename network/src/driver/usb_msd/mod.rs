//! USB mass-storage block driver scaffold.
//!
//! This module reserves the BlockDriver interface for the future ASM-backed
//! USB host + mass-storage transport path (read-focused milestone first).

use crate::driver::block_traits::{
    BlockCompletion, BlockDeviceInfo, BlockDriver, BlockDriverInit, BlockError,
};

extern "win64" {
    fn asm_usb_host_probe(mmio_base: u64) -> u32;
    fn asm_usb_host_reset(mmio_base: u64, tsc_freq: u64) -> u32;
}

/// USB mass-storage configuration.
#[derive(Debug, Clone)]
pub struct UsbMsdConfig {
    /// TSC frequency for timeout calculations.
    pub tsc_freq: u64,
    /// Optional DMA bounce buffer base (physical).
    pub dma_phys: u64,
    /// Optional DMA bounce buffer size.
    pub dma_size: usize,
}

/// USB mass-storage init errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbMsdInitError {
    InvalidConfig,
    ControllerInitFailed,
    DeviceEnumerationFailed,
    TransportInitFailed,
    NoMedia,
    CommandTimeout,
    IoError,
    NotImplemented,
}

impl core::fmt::Display for UsbMsdInitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidConfig => write!(f, "Invalid USB MSD configuration"),
            Self::ControllerInitFailed => write!(f, "USB controller init failed"),
            Self::DeviceEnumerationFailed => write!(f, "USB device enumeration failed"),
            Self::TransportInitFailed => write!(f, "USB mass-storage transport init failed"),
            Self::NoMedia => write!(f, "USB mass-storage media not present"),
            Self::CommandTimeout => write!(f, "USB mass-storage command timeout"),
            Self::IoError => write!(f, "USB mass-storage I/O error"),
            Self::NotImplemented => write!(f, "USB mass-storage driver not implemented yet"),
        }
    }
}

/// USB mass-storage driver state (scaffold).
pub struct UsbMsdDriver {
    mmio_base: u64,
    _tsc_freq: u64,
    info: BlockDeviceInfo,
}

impl UsbMsdDriver {
    /// Create a new USB mass-storage driver instance.
    ///
    /// Phase-1 scaffold returns NotImplemented until ASM primitives land.
    pub unsafe fn new(mmio_base: u64, config: UsbMsdConfig) -> Result<Self, UsbMsdInitError> {
        if mmio_base == 0 || config.tsc_freq == 0 {
            return Err(UsbMsdInitError::InvalidConfig);
        }

        if asm_usb_host_probe(mmio_base) != 0 {
            return Err(UsbMsdInitError::ControllerInitFailed);
        }

        if asm_usb_host_reset(mmio_base, config.tsc_freq) != 0 {
            return Err(UsbMsdInitError::ControllerInitFailed);
        }

        // Probe/reset path is wired, but BOT/SCSI read transport is not yet implemented.
        // Fail fast so higher layers skip this backend instead of selecting a non-functional disk.
        return Err(UsbMsdInitError::NotImplemented);
    }
}

impl BlockDriverInit for UsbMsdDriver {
    type Error = UsbMsdInitError;
    type Config = UsbMsdConfig;

    fn supported_vendors() -> &'static [u16] {
        &[]
    }

    fn supported_devices() -> &'static [u16] {
        &[]
    }

    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}

impl BlockDriver for UsbMsdDriver {
    fn info(&self) -> BlockDeviceInfo {
        self.info
    }

    fn can_submit(&self) -> bool {
        true
    }

    fn submit_read(
        &mut self,
        _sector: u64,
        _buffer_phys: u64,
        _num_sectors: u32,
        _request_id: u32,
    ) -> Result<(), BlockError> {
        let _ = self.mmio_base;
        Err(BlockError::DeviceNotReady)
    }

    fn submit_write(
        &mut self,
        _sector: u64,
        _buffer_phys: u64,
        _num_sectors: u32,
        _request_id: u32,
    ) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        None
    }

    fn notify(&mut self) {}
}
