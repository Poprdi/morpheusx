//! Sector alignment and calculation utilities

use crate::types::SECTOR_SIZE;

/// Align value to sector boundary (round up)
pub fn align_to_sector(value: usize) -> usize {
    (value + SECTOR_SIZE - 1) & !(SECTOR_SIZE - 1)
}

/// Convert byte offset to sector number
pub fn byte_to_sector(byte_offset: u64) -> u32 {
    (byte_offset / SECTOR_SIZE as u64) as u32
}

/// Convert sector number to byte offset
pub fn sector_to_byte(sector: u32) -> u64 {
    sector as u64 * SECTOR_SIZE as u64
}

/// Calculate number of sectors needed for byte count
pub fn sectors_for_bytes(byte_count: u32) -> u32 {
    byte_count.div_ceil(SECTOR_SIZE as u32)
}

/// Check if value is sector-aligned
pub fn is_sector_aligned(value: usize) -> bool {
    value & (SECTOR_SIZE - 1) == 0
}
