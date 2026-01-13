//! Unified BlockIo adapter for MorpheusX block devices.
//!
//! This module provides a `gpt_disk_io::BlockIo` implementation that wraps
//! our `UnifiedBlockDevice`, enabling filesystem operations on both VirtIO-blk
//! (QEMU) and AHCI SATA (real hardware like ThinkPad T450s).
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────┐
//! │     FAT32 / ISO9660 Filesystem         │
//! │        (uses BlockIo trait)            │
//! └───────────────────┬────────────────────┘
//!                     │ gpt_disk_io::BlockIo
//!                     ▼
//! ┌────────────────────────────────────────┐
//! │       UnifiedBlockIo (this)            │
//! │  Synchronous wrapper with DMA buffer   │
//! └───────────────────┬────────────────────┘
//!                     │ BlockDriver trait
//!                     ▼
//! ┌────────────────────────────────────────┐
//! │        UnifiedBlockDevice              │
//! │   ┌───────────────┬───────────────┐    │
//! │   │  VirtIO-blk   │  AHCI SATA    │    │
//! │   │    (QEMU)     │ (ThinkPad)    │    │
//! │   └───────────────┴───────────────┘    │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // Probe and create unified block device
//! let mut device = UnifiedBlockDevice::probe(&config)?;
//!
//! // Create BlockIo adapter for filesystem operations
//! let mut adapter = UnifiedBlockIo::new(&mut device, dma_buffer, dma_phys, timeout)?;
//!
//! // Now use with FAT32 or ISO9660 - works on QEMU or real hardware!
//! fat32_ops::read_file(&mut adapter, partition_start, "/vmlinuz")?;
//! ```

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

use crate::device::UnifiedBlockDevice;
use super::block_traits::{BlockDriver, BlockError};

/// Error type for unified BlockIo operations.
#[derive(Debug, Clone, Copy)]
pub enum UnifiedBlockIoError {
    /// Underlying block driver error
    DriverError(BlockError),
    /// Request timeout
    Timeout,
    /// Buffer alignment error
    BufferAlignment,
    /// Invalid operation
    InvalidOperation,
    /// Device not ready
    DeviceNotReady,
}

impl core::fmt::Display for UnifiedBlockIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DriverError(e) => write!(f, "Block driver error: {:?}", e),
            Self::Timeout => write!(f, "I/O timeout"),
            Self::BufferAlignment => write!(f, "Buffer alignment error"),
            Self::InvalidOperation => write!(f, "Invalid operation"),
            Self::DeviceNotReady => write!(f, "Block device not ready"),
        }
    }
}

/// Unified BlockIo adapter for MorpheusX block devices.
///
/// Provides synchronous block I/O by wrapping the `UnifiedBlockDevice`
/// and using a DMA-capable buffer for transfers. Works with both:
/// - **VirtIO-blk** (QEMU, cloud VMs)
/// - **AHCI SATA** (real hardware like ThinkPad T450s)
///
/// # Example
///
/// ```ignore
/// // Create unified device via probe
/// let mut device = UnifiedBlockDevice::probe(&config)?;
///
/// // Wrap with BlockIo adapter
/// let mut bio = UnifiedBlockIo::new(&mut device, dma_buf, dma_phys, timeout)?;
///
/// // Use with filesystem layer
/// let info = bio.block_size();
/// bio.read_blocks(Lba(0), &mut buffer)?;
/// ```
pub struct UnifiedBlockIo<'a> {
    /// The underlying unified block device
    device: &'a mut UnifiedBlockDevice,
    /// DMA buffer for transfers (must be physically contiguous)
    dma_buffer: &'a mut [u8],
    /// Physical address of DMA buffer
    dma_buffer_phys: u64,
    /// Next request ID
    next_request_id: u32,
    /// Timeout in TSC ticks
    timeout_ticks: u64,
}

impl<'a> UnifiedBlockIo<'a> {
    /// Maximum transfer size per request (64KB default)
    pub const MAX_TRANSFER_SIZE: usize = 64 * 1024;

    /// Create a new unified BlockIo adapter.
    ///
    /// # Arguments
    /// * `device` - Unified block device (VirtIO-blk or AHCI)
    /// * `dma_buffer` - DMA-capable buffer (must be at least MAX_TRANSFER_SIZE bytes)
    /// * `dma_buffer_phys` - Physical address of DMA buffer
    /// * `timeout_ticks` - Timeout for I/O operations in TSC ticks
    ///
    /// # Returns
    /// New adapter or error if buffer too small or device not ready
    pub fn new(
        device: &'a mut UnifiedBlockDevice,
        dma_buffer: &'a mut [u8],
        dma_buffer_phys: u64,
        timeout_ticks: u64,
    ) -> Result<Self, UnifiedBlockIoError> {
        if dma_buffer.len() < Self::MAX_TRANSFER_SIZE {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        if !device.is_ready() {
            return Err(UnifiedBlockIoError::DeviceNotReady);
        }

        Ok(Self {
            device,
            dma_buffer,
            dma_buffer_phys,
            next_request_id: 1,
            timeout_ticks,
        })
    }

    /// Get the underlying device type as a string.
    pub fn device_type(&self) -> &'static str {
        self.device.driver_type()
    }

    /// Wait for a specific request to complete.
    fn wait_for_completion(&mut self, request_id: u32) -> Result<(), UnifiedBlockIoError> {
        let start = crate::mainloop::runner::get_tsc();

        loop {
            // Poll for completions
            if let Some(completion) = self.device.poll_completion() {
                if completion.request_id == request_id {
                    if completion.status == 0 {
                        return Ok(());
                    } else {
                        return Err(UnifiedBlockIoError::DriverError(BlockError::IoError));
                    }
                }
                // Not our completion, continue polling
            }

            // Check timeout
            let now = crate::mainloop::runner::get_tsc();
            if now.wrapping_sub(start) > self.timeout_ticks {
                return Err(UnifiedBlockIoError::Timeout);
            }

            core::hint::spin_loop();
        }
    }

    /// Perform a synchronous read.
    fn sync_read(
        &mut self,
        sector: u64,
        num_sectors: u32,
        dst: &mut [u8],
    ) -> Result<(), UnifiedBlockIoError> {
        let info = self.device.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        // Drain any pending completions
        while self.device.poll_completion().is_some() {}

        // Submit read request
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.device
            .submit_read(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(UnifiedBlockIoError::DriverError)?;

        self.device.notify();

        // Wait for completion
        self.wait_for_completion(request_id)?;

        // Copy data to destination
        dst.copy_from_slice(&self.dma_buffer[..bytes_needed]);

        Ok(())
    }

    /// Perform a synchronous write.
    fn sync_write(
        &mut self,
        sector: u64,
        num_sectors: u32,
        src: &[u8],
    ) -> Result<(), UnifiedBlockIoError> {
        let info = self.device.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        // Copy data to DMA buffer
        self.dma_buffer[..bytes_needed].copy_from_slice(src);

        // Drain any pending completions
        while self.device.poll_completion().is_some() {}

        // Submit write request
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.device
            .submit_write(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(UnifiedBlockIoError::DriverError)?;

        self.device.notify();

        // Wait for completion
        self.wait_for_completion(request_id)
    }
}

impl<'a> BlockIo for UnifiedBlockIo<'a> {
    type Error = UnifiedBlockIoError;

    fn block_size(&self) -> BlockSize {
        let info = self.device.info();
        BlockSize::new(info.sector_size).expect("valid sector size")
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        let info = self.device.info();
        Ok(info.total_sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let info = self.device.info();
        let sector_size = info.sector_size as usize;

        if dst.len() % sector_size != 0 {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        let total_sectors = (dst.len() / sector_size) as u32;
        let max_sectors_per_request = (Self::MAX_TRANSFER_SIZE / sector_size) as u32;

        let mut current_sector = start_lba.0;
        let mut remaining = total_sectors;
        let mut offset = 0;

        while remaining > 0 {
            let chunk_sectors = remaining.min(max_sectors_per_request);
            let chunk_bytes = chunk_sectors as usize * sector_size;

            self.sync_read(
                current_sector,
                chunk_sectors,
                &mut dst[offset..offset + chunk_bytes],
            )?;

            current_sector += chunk_sectors as u64;
            remaining -= chunk_sectors;
            offset += chunk_bytes;
        }

        Ok(())
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let info = self.device.info();
        let sector_size = info.sector_size as usize;

        if src.len() % sector_size != 0 {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        let total_sectors = (src.len() / sector_size) as u32;
        let max_sectors_per_request = (Self::MAX_TRANSFER_SIZE / sector_size) as u32;

        let mut current_sector = start_lba.0;
        let mut remaining = total_sectors;
        let mut offset = 0;

        while remaining > 0 {
            let chunk_sectors = remaining.min(max_sectors_per_request);
            let chunk_bytes = chunk_sectors as usize * sector_size;

            self.sync_write(
                current_sector,
                chunk_sectors,
                &src[offset..offset + chunk_bytes],
            )?;

            current_sector += chunk_sectors as u64;
            remaining -= chunk_sectors;
            offset += chunk_bytes;
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.device.flush().map_err(UnifiedBlockIoError::DriverError)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GENERIC BLOCKIO ADAPTER
// ═══════════════════════════════════════════════════════════════════════════

/// Generic BlockIo adapter for any `BlockDriver` implementation.
///
/// This allows creating a synchronous BlockIo wrapper around any driver
/// that implements the `BlockDriver` trait, not just `UnifiedBlockDevice`.
///
/// # Example
///
/// ```ignore
/// // Works with AHCI driver directly
/// let mut ahci = AhciDriver::new(abar, config)?;
/// let mut bio = GenericBlockIo::new(&mut ahci, dma_buf, dma_phys, timeout)?;
///
/// // Or VirtIO-blk directly
/// let mut virtio = VirtioBlkDriver::new(mmio_base, config)?;
/// let mut bio = GenericBlockIo::new(&mut virtio, dma_buf, dma_phys, timeout)?;
/// ```
pub struct GenericBlockIo<'a, D: BlockDriver> {
    /// The underlying block driver
    driver: &'a mut D,
    /// DMA buffer for transfers (must be physically contiguous)
    dma_buffer: &'a mut [u8],
    /// Physical address of DMA buffer
    dma_buffer_phys: u64,
    /// Next request ID
    next_request_id: u32,
    /// Timeout in TSC ticks
    timeout_ticks: u64,
}

impl<'a, D: BlockDriver> GenericBlockIo<'a, D> {
    /// Maximum transfer size per request (64KB default)
    pub const MAX_TRANSFER_SIZE: usize = 64 * 1024;

    /// Create a new generic BlockIo adapter.
    pub fn new(
        driver: &'a mut D,
        dma_buffer: &'a mut [u8],
        dma_buffer_phys: u64,
        timeout_ticks: u64,
    ) -> Result<Self, UnifiedBlockIoError> {
        if dma_buffer.len() < Self::MAX_TRANSFER_SIZE {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        Ok(Self {
            driver,
            dma_buffer,
            dma_buffer_phys,
            next_request_id: 1,
            timeout_ticks,
        })
    }

    /// Wait for a specific request to complete.
    fn wait_for_completion(&mut self, request_id: u32) -> Result<(), UnifiedBlockIoError> {
        let start = crate::mainloop::runner::get_tsc();

        loop {
            if let Some(completion) = self.driver.poll_completion() {
                if completion.request_id == request_id {
                    if completion.status == 0 {
                        return Ok(());
                    } else {
                        return Err(UnifiedBlockIoError::DriverError(BlockError::IoError));
                    }
                }
            }

            let now = crate::mainloop::runner::get_tsc();
            if now.wrapping_sub(start) > self.timeout_ticks {
                return Err(UnifiedBlockIoError::Timeout);
            }

            core::hint::spin_loop();
        }
    }

    /// Perform a synchronous read.
    fn sync_read(
        &mut self,
        sector: u64,
        num_sectors: u32,
        dst: &mut [u8],
    ) -> Result<(), UnifiedBlockIoError> {
        let info = self.driver.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        while self.driver.poll_completion().is_some() {}

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.driver
            .submit_read(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(UnifiedBlockIoError::DriverError)?;

        self.driver.notify();

        self.wait_for_completion(request_id)?;

        dst.copy_from_slice(&self.dma_buffer[..bytes_needed]);

        Ok(())
    }

    /// Perform a synchronous write.
    fn sync_write(
        &mut self,
        sector: u64,
        num_sectors: u32,
        src: &[u8],
    ) -> Result<(), UnifiedBlockIoError> {
        let info = self.driver.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        self.dma_buffer[..bytes_needed].copy_from_slice(src);

        while self.driver.poll_completion().is_some() {}

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.driver
            .submit_write(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(UnifiedBlockIoError::DriverError)?;

        self.driver.notify();

        self.wait_for_completion(request_id)
    }
}

impl<'a, D: BlockDriver> BlockIo for GenericBlockIo<'a, D> {
    type Error = UnifiedBlockIoError;

    fn block_size(&self) -> BlockSize {
        let info = self.driver.info();
        BlockSize::new(info.sector_size).expect("valid sector size")
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        let info = self.driver.info();
        Ok(info.total_sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let info = self.driver.info();
        let sector_size = info.sector_size as usize;

        if dst.len() % sector_size != 0 {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        let total_sectors = (dst.len() / sector_size) as u32;
        let max_sectors_per_request = (Self::MAX_TRANSFER_SIZE / sector_size) as u32;

        let mut current_sector = start_lba.0;
        let mut remaining = total_sectors;
        let mut offset = 0;

        while remaining > 0 {
            let chunk_sectors = remaining.min(max_sectors_per_request);
            let chunk_bytes = chunk_sectors as usize * sector_size;

            self.sync_read(
                current_sector,
                chunk_sectors,
                &mut dst[offset..offset + chunk_bytes],
            )?;

            current_sector += chunk_sectors as u64;
            remaining -= chunk_sectors;
            offset += chunk_bytes;
        }

        Ok(())
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let info = self.driver.info();
        let sector_size = info.sector_size as usize;

        if src.len() % sector_size != 0 {
            return Err(UnifiedBlockIoError::BufferAlignment);
        }

        let total_sectors = (src.len() / sector_size) as u32;
        let max_sectors_per_request = (Self::MAX_TRANSFER_SIZE / sector_size) as u32;

        let mut current_sector = start_lba.0;
        let mut remaining = total_sectors;
        let mut offset = 0;

        while remaining > 0 {
            let chunk_sectors = remaining.min(max_sectors_per_request);
            let chunk_bytes = chunk_sectors as usize * sector_size;

            self.sync_write(
                current_sector,
                chunk_sectors,
                &src[offset..offset + chunk_bytes],
            )?;

            current_sector += chunk_sectors as u64;
            remaining -= chunk_sectors;
            offset += chunk_bytes;
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.driver.flush().map_err(UnifiedBlockIoError::DriverError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here with mock driver
}
