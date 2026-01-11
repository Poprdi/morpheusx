//! BlockIo adapter for VirtIO-blk driver.
//!
//! This module provides a `gpt_disk_io::BlockIo` implementation that wraps
//! our VirtIO-blk driver. This allows using the FAT32 and ISO9660 filesystem
//! implementations post-ExitBootServices.
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
//! │       VirtioBlkBlockIo (this)          │
//! │  Synchronous wrapper with DMA buffer   │
//! └───────────────────┬────────────────────┘
//!                     │ BlockDriver trait
//!                     ▼
//! ┌────────────────────────────────────────┐
//! │        VirtioBlkDriver                 │
//! │    (async submit/poll interface)       │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! let mut blk_driver = VirtioBlkDriver::new(mmio_base, config)?;
//! let mut adapter = VirtioBlkBlockIo::new(&mut blk_driver, dma_buffer)?;
//!
//! // Now use with FAT32
//! fat32_ops::read_file(&mut adapter, partition_start, "/vmlinuz")?;
//! ```

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

use super::block_traits::{BlockDriver, BlockError};
use super::virtio_blk::VirtioBlkDriver;

/// Error type for BlockIo operations.
#[derive(Debug, Clone, Copy)]
pub enum BlockIoError {
    /// Underlying block driver error
    DriverError(BlockError),
    /// Request timeout
    Timeout,
    /// Buffer alignment error
    BufferAlignment,
    /// Invalid operation
    InvalidOperation,
}

impl core::fmt::Display for BlockIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DriverError(e) => write!(f, "Block driver error: {:?}", e),
            Self::Timeout => write!(f, "I/O timeout"),
            Self::BufferAlignment => write!(f, "Buffer alignment error"),
            Self::InvalidOperation => write!(f, "Invalid operation"),
        }
    }
}

/// BlockIo adapter for VirtIO-blk driver.
///
/// Provides synchronous block I/O by wrapping the async VirtIO-blk driver
/// and using a DMA-capable buffer for transfers.
pub struct VirtioBlkBlockIo<'a> {
    /// The underlying VirtIO-blk driver
    driver: &'a mut VirtioBlkDriver,
    /// DMA buffer for transfers (must be physically contiguous)
    dma_buffer: &'a mut [u8],
    /// Physical address of DMA buffer
    dma_buffer_phys: u64,
    /// Next request ID
    next_request_id: u32,
    /// Timeout in TSC ticks
    timeout_ticks: u64,
}

impl<'a> VirtioBlkBlockIo<'a> {
    /// Maximum transfer size per request (64KB default)
    pub const MAX_TRANSFER_SIZE: usize = 64 * 1024;

    /// Create a new BlockIo adapter.
    ///
    /// # Arguments
    /// * `driver` - VirtIO-blk driver
    /// * `dma_buffer` - DMA-capable buffer (must be at least MAX_TRANSFER_SIZE bytes)
    /// * `dma_buffer_phys` - Physical address of DMA buffer
    /// * `timeout_ticks` - Timeout for I/O operations in TSC ticks
    ///
    /// # Returns
    /// New adapter or error if buffer too small
    pub fn new(
        driver: &'a mut VirtioBlkDriver,
        dma_buffer: &'a mut [u8],
        dma_buffer_phys: u64,
        timeout_ticks: u64,
    ) -> Result<Self, BlockIoError> {
        if dma_buffer.len() < Self::MAX_TRANSFER_SIZE {
            return Err(BlockIoError::BufferAlignment);
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
    fn wait_for_completion(&mut self, request_id: u32) -> Result<(), BlockIoError> {
        let start = crate::mainloop::runner::get_tsc();

        loop {
            // Poll for completions
            if let Some(completion) = self.driver.poll_completion() {
                if completion.request_id == request_id {
                    if completion.status == 0 {
                        return Ok(());
                    } else {
                        return Err(BlockIoError::DriverError(BlockError::IoError));
                    }
                }
                // Not our completion, continue polling
            }

            // Check timeout
            let now = crate::mainloop::runner::get_tsc();
            if now.wrapping_sub(start) > self.timeout_ticks {
                return Err(BlockIoError::Timeout);
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
    ) -> Result<(), BlockIoError> {
        let info = self.driver.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(BlockIoError::BufferAlignment);
        }

        // Drain any pending completions
        while self.driver.poll_completion().is_some() {}

        // Submit read request
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.driver
            .submit_read(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(BlockIoError::DriverError)?;

        self.driver.notify();

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
    ) -> Result<(), BlockIoError> {
        let info = self.driver.info();
        let bytes_needed = num_sectors as usize * info.sector_size as usize;

        if bytes_needed > self.dma_buffer.len() {
            return Err(BlockIoError::BufferAlignment);
        }

        // Copy data to DMA buffer
        self.dma_buffer[..bytes_needed].copy_from_slice(src);

        // Drain any pending completions
        while self.driver.poll_completion().is_some() {}

        // Submit write request
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        self.driver
            .submit_write(sector, self.dma_buffer_phys, num_sectors, request_id)
            .map_err(BlockIoError::DriverError)?;

        self.driver.notify();

        // Wait for completion
        self.wait_for_completion(request_id)
    }
}

impl<'a> BlockIo for VirtioBlkBlockIo<'a> {
    type Error = BlockIoError;

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
            return Err(BlockIoError::BufferAlignment);
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
            return Err(BlockIoError::BufferAlignment);
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
        self.driver.flush().map_err(BlockIoError::DriverError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here with mock driver
}
