//! Raw memory read utilities for PE parsing

#[inline]
pub unsafe fn read_u16(base: *const u8, offset: usize) -> u16 {
    u16::from_le_bytes([*base.add(offset), *base.add(offset + 1)])
}

#[inline]
pub unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    u32::from_le_bytes([
        *base.add(offset),
        *base.add(offset + 1),
        *base.add(offset + 2),
        *base.add(offset + 3),
    ])
}

#[inline]
pub unsafe fn read_u64(base: *const u8, offset: usize) -> u64 {
    u64::from_le_bytes([
        *base.add(offset),
        *base.add(offset + 1),
        *base.add(offset + 2),
        *base.add(offset + 3),
        *base.add(offset + 4),
        *base.add(offset + 5),
        *base.add(offset + 6),
        *base.add(offset + 7),
    ])
}
