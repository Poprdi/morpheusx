// UEFI Block I/O Protocol for disk access

#[repr(C)]
pub struct BlockIoProtocol {
    pub revision: u64,
    pub media: *const BlockIoMedia,
    pub reset: extern "efiapi" fn(*mut BlockIoProtocol, bool) -> usize,
    pub read_blocks: extern "efiapi" fn(
        *mut BlockIoProtocol,
        u32,     // MediaId
        u64,     // LBA
        usize,   // BufferSize
        *mut u8, // Buffer
    ) -> usize,
    pub write_blocks: extern "efiapi" fn(*mut BlockIoProtocol, u32, u64, usize, *const u8) -> usize,
    pub flush_blocks: extern "efiapi" fn(*mut BlockIoProtocol) -> usize,
}

#[repr(C)]
pub struct BlockIoMedia {
    pub media_id: u32,
    pub removable_media: bool,
    pub media_present: bool,
    pub logical_partition: bool,
    pub read_only: bool,
    pub write_caching: bool,
    pub block_size: u32,
    pub io_align: u32,
    pub last_block: u64,
    // UEFI 2.0+
    pub lowest_aligned_lba: u64,
    pub logical_blocks_per_physical_block: u32,
    // UEFI 2.1+
    pub optimal_transfer_length_granularity: u32,
}

pub const EFI_BLOCK_IO_PROTOCOL_GUID: [u8; 16] = [
    0x21, 0x5b, 0x4e, 0x96, 0x59, 0x64, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

impl BlockIoProtocol {
    pub fn read_sectors(&mut self, lba: u64, count: u64, buffer: &mut [u8]) -> Result<(), usize> {
        unsafe {
            let media = &*self.media;
            let block_size = media.block_size as usize;
            let total_size = block_size * count as usize;

            if buffer.len() < total_size {
                return Err(1);
            }

            let status =
                (self.read_blocks)(self, media.media_id, lba, total_size, buffer.as_mut_ptr());

            if status == 0 {
                Ok(())
            } else {
                Err(status)
            }
        }
    }

    pub fn write_sectors(&mut self, lba: u64, count: u64, buffer: &[u8]) -> Result<(), usize> {
        unsafe {
            let media = &*self.media;

            if media.read_only {
                return Err(8); // Write protected
            }

            let block_size = media.block_size as usize;
            let total_size = block_size * count as usize;

            if buffer.len() < total_size {
                return Err(1);
            }

            let status =
                (self.write_blocks)(self, media.media_id, lba, total_size, buffer.as_ptr());

            if status == 0 {
                Ok(())
            } else {
                Err(status)
            }
        }
    }
}
