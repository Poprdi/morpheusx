//! UEFI BlockIo to gpt_disk_io::BlockIo adapter
//!
//! Provides a wrapper that implements the `gpt_disk_io::BlockIo` trait
//! for UEFI BlockIoProtocol, enabling use with GPT and ISO libraries.

use super::block_io::BlockIoProtocol;
use core::fmt;
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

/// Error type for UEFI block I/O operations
#[derive(Debug, Clone, Copy)]
pub struct UefiBlockIoError(pub usize);

impl fmt::Display for UefiBlockIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UEFI BlockIo error: {}", self.0)
    }
}

/// Wrapper around UEFI BlockIoProtocol implementing gpt_disk_io::BlockIo
pub struct UefiBlockIo {
    protocol: *mut BlockIoProtocol,
    block_size: BlockSize,
    num_blocks: u64,
}

impl UefiBlockIo {
    /// Create a new wrapper
    ///
    /// # Safety
    /// The protocol pointer must be valid for the lifetime of this wrapper.
    pub unsafe fn new(protocol: *mut BlockIoProtocol) -> Self {
        let media = &*(*protocol).media;
        let block_size = BlockSize::new(media.block_size).unwrap_or(BlockSize::BS_512);
        let num_blocks = media.last_block + 1;

        Self {
            protocol,
            block_size,
            num_blocks,
        }
    }

    /// Get the underlying protocol pointer
    pub fn protocol(&self) -> *mut BlockIoProtocol {
        self.protocol
    }

    /// Get block size as u32
    pub fn block_size_bytes(&self) -> u32 {
        self.block_size.to_u32().unwrap_or(512)
    }

    /// Get total number of blocks
    pub fn total_blocks(&self) -> u64 {
        self.num_blocks
    }
}

impl BlockIo for UefiBlockIo {
    type Error = UefiBlockIoError;

    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.num_blocks)
    }

    fn read_blocks(&mut self, start_lba: Lba, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let block_size = self.block_size.to_u64().unwrap_or(512);
        let num_blocks = buffer.len() as u64 / block_size;

        // SAFETY: Protocol pointer is valid (guaranteed by constructor)
        unsafe {
            let protocol = &mut *self.protocol;
            protocol
                .read_sectors(start_lba.0, num_blocks, buffer)
                .map_err(UefiBlockIoError)
        }
    }

    fn write_blocks(&mut self, start_lba: Lba, buffer: &[u8]) -> Result<(), Self::Error> {
        let block_size = self.block_size.to_u64().unwrap_or(512);
        let num_blocks = buffer.len() as u64 / block_size;

        // SAFETY: Protocol pointer is valid (guaranteed by constructor)
        unsafe {
            let protocol = &mut *self.protocol;
            protocol
                .write_sectors(start_lba.0, num_blocks, buffer)
                .map_err(UefiBlockIoError)
        }
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        // SAFETY: Protocol pointer is valid
        unsafe {
            let protocol = &mut *self.protocol;
            let status = ((*protocol).flush_blocks)(protocol);
            if status == 0 {
                Ok(())
            } else {
                Err(UefiBlockIoError(status))
            }
        }
    }
}
