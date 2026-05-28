//! Test helpers: in-memory `BlockIo` and a hand-rolled minimal ISO image.

pub mod builder;
pub use builder::IsoBuilder;

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use std::io;

#[derive(Debug, Clone)]
pub struct MemoryBlockDevice {
    pub data: Vec<u8>,
    pub block_size: usize,
}

impl MemoryBlockDevice {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            block_size: 2048,
        }
    }

    pub fn from_file(path: &str) -> io::Result<Self> {
        let data = std::fs::read(path)?;
        Ok(Self::new(data))
    }

    /// 64-sector image: PVD@16, terminator@17, root dir@18 with "." and "..".
    pub fn create_minimal_iso() -> Self {
        let mut data = vec![0u8; 64 * 2048];

        // PVD at sector 16 (ISO 9660 §8.4).
        let pvd_offset = 16 * 2048;
        data[pvd_offset] = 1;
        data[pvd_offset + 1..pvd_offset + 6].copy_from_slice(b"CD001");
        data[pvd_offset + 6] = 1;
        data[pvd_offset + 8..pvd_offset + 19].copy_from_slice(b"TEST SYSTEM");
        data[pvd_offset + 40..pvd_offset + 51].copy_from_slice(b"TEST VOLUME");

        // Both-endian volume_space_size and logical_block_size.
        data[pvd_offset + 80..pvd_offset + 84].copy_from_slice(&64u32.to_le_bytes());
        data[pvd_offset + 84..pvd_offset + 88].copy_from_slice(&64u32.to_be_bytes());
        data[pvd_offset + 128..pvd_offset + 130].copy_from_slice(&2048u16.to_le_bytes());
        data[pvd_offset + 130..pvd_offset + 132].copy_from_slice(&2048u16.to_be_bytes());

        // Root directory record embedded at PVD+156.
        let root_offset = pvd_offset + 156;
        data[root_offset] = 34;
        data[root_offset + 1] = 0;
        data[root_offset + 2..root_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[root_offset + 6..root_offset + 10].copy_from_slice(&18u32.to_be_bytes());
        data[root_offset + 10..root_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[root_offset + 14..root_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
        data[root_offset + 25] = 0x02;
        data[root_offset + 32] = 1;
        data[root_offset + 33] = 0x00;

        // Set terminator at sector 17.
        let term_offset = 17 * 2048;
        data[term_offset] = 255;
        data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
        data[term_offset + 6] = 1;

        // Root directory at sector 18: "." (0x00) and ".." (0x01).
        let root_dir_offset = 18 * 2048;
        data[root_dir_offset] = 34;
        data[root_dir_offset + 2..root_dir_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[root_dir_offset + 6..root_dir_offset + 10].copy_from_slice(&18u32.to_be_bytes());
        data[root_dir_offset + 10..root_dir_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[root_dir_offset + 14..root_dir_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
        data[root_dir_offset + 25] = 0x02;
        data[root_dir_offset + 32] = 1;
        data[root_dir_offset + 33] = 0x00;

        let parent_offset = root_dir_offset + 34;
        data[parent_offset] = 34;
        data[parent_offset + 2..parent_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[parent_offset + 6..parent_offset + 10].copy_from_slice(&18u32.to_be_bytes());
        data[parent_offset + 10..parent_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[parent_offset + 14..parent_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
        data[parent_offset + 25] = 0x02;
        data[parent_offset + 32] = 1;
        data[parent_offset + 33] = 0x01;

        Self::new(data)
    }
}

impl BlockIo for MemoryBlockDevice {
    type Error = io::Error;

    fn block_size(&self) -> BlockSize {
        BlockSize::new(self.block_size as u32).expect("valid block size")
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok((self.data.len() / self.block_size) as u64)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 as usize * self.block_size;
        if offset + dst.len() > self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read beyond end of device",
            ));
        }
        dst.copy_from_slice(&self.data[offset..offset + dst.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 as usize * self.block_size;
        if offset + src.len() > self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write beyond end of device",
            ));
        }
        self.data[offset..offset + src.len()].copy_from_slice(src);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
