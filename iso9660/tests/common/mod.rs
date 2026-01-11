//! Common test utilities and mock block devices

pub mod builder;
pub use builder::IsoBuilder;

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use std::io;

/// In-memory block device for testing
#[derive(Debug, Clone)]
pub struct MemoryBlockDevice {
    pub data: Vec<u8>,
    pub block_size: usize,
}

impl MemoryBlockDevice {
    /// Create a new memory block device from raw data
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            block_size: 2048, // ISO9660 sector size
        }
    }

    /// Create from a file path
    #[allow(dead_code)]
    pub fn from_file(path: &str) -> io::Result<Self> {
        let data = std::fs::read(path)?;
        Ok(Self::new(data))
    }

    /// Create a minimal valid ISO9660 volume for testing
    pub fn create_minimal_iso() -> Self {
        let mut data = vec![0u8; 64 * 2048]; // 64 sectors

        // System area (sectors 0-15) - all zeros

        // Primary Volume Descriptor (sector 16)
        let pvd_offset = 16 * 2048;

        // Type code (1 = primary)
        data[pvd_offset] = 1;

        // Standard identifier "CD001"
        data[pvd_offset + 1..pvd_offset + 6].copy_from_slice(b"CD001");

        // Version (1)
        data[pvd_offset + 6] = 1;

        // System identifier (32 bytes) - "TEST SYSTEM"
        data[pvd_offset + 8..pvd_offset + 19].copy_from_slice(b"TEST SYSTEM");

        // Volume identifier (32 bytes) - "TEST VOLUME"
        data[pvd_offset + 40..pvd_offset + 51].copy_from_slice(b"TEST VOLUME");

        // Volume space size (both byte orders) - 64 sectors
        data[pvd_offset + 80..pvd_offset + 84].copy_from_slice(&64u32.to_le_bytes());
        data[pvd_offset + 84..pvd_offset + 88].copy_from_slice(&64u32.to_be_bytes());

        // Logical block size (both byte orders) - 2048 bytes
        data[pvd_offset + 128..pvd_offset + 130].copy_from_slice(&2048u16.to_le_bytes());
        data[pvd_offset + 130..pvd_offset + 132].copy_from_slice(&2048u16.to_be_bytes());

        // Root directory record (at offset 156, 34 bytes)
        let root_offset = pvd_offset + 156;

        // Length of directory record (34)
        data[root_offset] = 34;

        // Extended attribute record length (0)
        data[root_offset + 1] = 0;

        // Extent location (sector 18)
        data[root_offset + 2..root_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[root_offset + 6..root_offset + 10].copy_from_slice(&18u32.to_be_bytes());

        // Data length (2048 = 1 sector)
        data[root_offset + 10..root_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[root_offset + 14..root_offset + 18].copy_from_slice(&2048u32.to_be_bytes());

        // File flags (0x02 = directory)
        data[root_offset + 25] = 0x02;

        // File identifier length (1)
        data[root_offset + 32] = 1;

        // File identifier (0x00 for root)
        data[root_offset + 33] = 0x00;

        // Volume Descriptor Set Terminator (sector 17)
        let term_offset = 17 * 2048;
        data[term_offset] = 255; // Type = terminator
        data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
        data[term_offset + 6] = 1;

        // Root directory (sector 18)
        let root_dir_offset = 18 * 2048;

        // "." entry (self)
        data[root_dir_offset] = 34; // Length
        data[root_dir_offset + 2..root_dir_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[root_dir_offset + 6..root_dir_offset + 10].copy_from_slice(&18u32.to_be_bytes());
        data[root_dir_offset + 10..root_dir_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[root_dir_offset + 14..root_dir_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
        data[root_dir_offset + 25] = 0x02; // Directory flag
        data[root_dir_offset + 32] = 1; // Name length
        data[root_dir_offset + 33] = 0x00; // Name: 0x00

        // ".." entry (parent = self for root)
        let parent_offset = root_dir_offset + 34;
        data[parent_offset] = 34; // Length
        data[parent_offset + 2..parent_offset + 6].copy_from_slice(&18u32.to_le_bytes());
        data[parent_offset + 6..parent_offset + 10].copy_from_slice(&18u32.to_be_bytes());
        data[parent_offset + 10..parent_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
        data[parent_offset + 14..parent_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
        data[parent_offset + 25] = 0x02; // Directory flag
        data[parent_offset + 32] = 1; // Name length
        data[parent_offset + 33] = 0x01; // Name: 0x01

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

/// Helper to create a test file entry (simple injection without proper directory update for now)
#[allow(dead_code)]
pub fn create_test_file(
    device: &mut MemoryBlockDevice,
    _parent_lba: u32,
    _name: &str,
    content: &[u8],
    file_lba: u32,
) {
    // Write file content to specified LBA
    let file_offset = file_lba as usize * 2048;
    let content_len = content.len();
    if file_offset + content_len <= device.data.len() {
        device.data[file_offset..file_offset + content_len].copy_from_slice(content);
    }
}
