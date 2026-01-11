// GPT operations using gpt-disk-rs

use super::{find_free_space, GptError};
use gpt_disk_io::BlockIo;

/// Scan disk for GPT and populate partition table
pub fn align_lba(lba: u64, block_size_bytes: u32) -> u64 {
    let alignment = (1024 * 1024) / block_size_bytes as u64; // 1MB alignment
    lba.div_ceil(alignment) * alignment
}

/// Calculate size in LBA from MB
pub fn mb_to_lba(size_mb: u64, block_size_bytes: u32) -> u64 {
    (size_mb * 1024 * 1024) / block_size_bytes as u64
}

/// Calculate total free space on disk in MB
pub fn calculate_total_free_space<B: BlockIo>(
    block_io: B,
    block_size_bytes: usize,
) -> Result<u64, GptError> {
    let free_regions = find_free_space(block_io, block_size_bytes)?;

    let mut total_free_lba = 0u64;
    for region in free_regions.iter().flatten() {
        total_free_lba += region.size_lba();
    }

    // Convert to MB
    Ok((total_free_lba * 512) / (1024 * 1024))
}
