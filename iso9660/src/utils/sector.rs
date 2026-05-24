//! Sector alignment helpers.

use crate::types::SECTOR_SIZE;

/// Round `value` up to the next 2048-byte boundary.
pub fn align_to_sector(value: usize) -> usize {
    (value + SECTOR_SIZE - 1) & !(SECTOR_SIZE - 1)
}

/// Sector containing `byte_offset`.
pub fn byte_to_sector(byte_offset: u64) -> u32 {
    (byte_offset / SECTOR_SIZE as u64) as u32
}

/// Start byte of sector `sector`.
pub fn sector_to_byte(sector: u32) -> u64 {
    sector as u64 * SECTOR_SIZE as u64
}

/// Sectors required to cover `byte_count`, rounded up.
pub fn sectors_for_bytes(byte_count: u32) -> u32 {
    byte_count.div_ceil(SECTOR_SIZE as u32)
}

/// True if `value` is a multiple of the sector size.
pub fn is_sector_aligned(value: usize) -> bool {
    value & (SECTOR_SIZE - 1) == 0
}
