use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

#[derive(Debug, Clone, Copy)]
pub struct MemIoError;

impl core::fmt::Display for MemIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory block I/O error")
    }
}

/// In-memory block device backed by a contiguous physical region.
///
/// Identity-mapped: the kernel can dereference `base` directly.
/// Zero-copy reads/writes via memcpy. Never fails.
pub struct MemBlockDevice {
    base: *mut u8,
    sectors: u64,
    sector_size: u32,
}

unsafe impl Send for MemBlockDevice {}
unsafe impl Sync for MemBlockDevice {}

impl MemBlockDevice {
    /// Wrap a raw memory region as a block device.
    ///
    /// # Safety
    /// `base` must point to `size` bytes of valid, identity-mapped memory
    /// that remains live for the device's lifetime.
    pub unsafe fn new(base: *mut u8, size: usize, sector_size: u32) -> Self {
        Self {
            base,
            sectors: size as u64 / sector_size as u64,
            sector_size,
        }
    }

    pub fn base(&self) -> *mut u8 { self.base }
    pub fn total_bytes(&self) -> u64 { self.sectors * self.sector_size as u64 }
}

impl BlockIo for MemBlockDevice {
    type Error = MemIoError;

    fn block_size(&self) -> BlockSize {
        BlockSize::new(self.sector_size).unwrap()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * self.sector_size as u64;
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.base.add(offset as usize),
                dst.as_mut_ptr(),
                dst.len(),
            );
        }
        Ok(())
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * self.sector_size as u64;
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr(),
                self.base.add(offset as usize),
                src.len(),
            );
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
