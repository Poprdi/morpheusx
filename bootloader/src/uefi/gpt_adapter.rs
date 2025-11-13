// Adapter to use UEFI BlockIoProtocol with gpt_disk_io

use crate::uefi::block_io::BlockIoProtocol;
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use core::fmt;

pub struct UefiBlockIoAdapter<'a> {
    protocol: &'a mut BlockIoProtocol,
    block_size: BlockSize,
}

impl<'a> UefiBlockIoAdapter<'a> {
    pub fn new(protocol: &'a mut BlockIoProtocol) -> Result<Self, AdapterError> {
        let media = unsafe { &*protocol.media };
        
        let block_size = match media.block_size {
            512 => BlockSize::BS_512,
            4096 => BlockSize::BS_4096,
            _ => return Err(AdapterError::UnsupportedBlockSize(media.block_size)),
        };
        
        Ok(Self {
            protocol,
            block_size,
        })
    }
}

#[derive(Debug)]
pub enum AdapterError {
    UnsupportedBlockSize(u32),
    UefiError(usize),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBlockSize(size) => {
                write!(f, "Unsupported block size: {}", size)
            }
            Self::UefiError(code) => {
                write!(f, "UEFI error: {}", code)
            }
        }
    }
}

impl BlockIo for UefiBlockIoAdapter<'_> {
    type Error = AdapterError;

    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        let media = unsafe { &*self.protocol.media };
        Ok(media.last_block + 1)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        self.block_size.assert_valid_block_buffer(dst);
        
        let num_blocks = dst.len() / self.block_size.to_usize().unwrap();
        self.protocol
            .read_sectors(start_lba.to_u64(), num_blocks as u64, dst)
            .map_err(AdapterError::UefiError)
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        self.block_size.assert_valid_block_buffer(src);
        
        let num_blocks = src.len() / self.block_size.to_usize().unwrap();
        self.protocol
            .write_sectors(start_lba.to_u64(), num_blocks as u64, src)
            .map_err(AdapterError::UefiError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        // UEFI doesn't have explicit flush for BlockIO, writes are synchronous
        Ok(())
    }
}
