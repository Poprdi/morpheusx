//! Block bitmap: 1 bit per 4 KiB data block; 1 = allocated.
//! Stored contiguously at `superblock.bitmap_start`.
//! One block (4 KiB) maps 32768 data blocks (128 MiB).

use crate::error::HelixError;
use crate::types::BLOCK_SIZE;
use alloc::vec;
use alloc::vec::Vec;

pub struct BlockBitmap {
    bits: Vec<u8>,
    total_blocks: u64,
    free_count: u64,
    /// Starting index for next allocation scan.
    search_hint: u64,
}

impl BlockBitmap {
    pub fn new(total_blocks: u64) -> Self {
        let byte_count = total_blocks.div_ceil(8) as usize;
        Self {
            bits: vec![0u8; byte_count],
            total_blocks,
            free_count: total_blocks,
            search_hint: 0,
        }
    }

    /// Load bitmap from raw bytes (read from disk).
    pub fn from_bytes(data: &[u8], total_blocks: u64) -> Self {
        let byte_count = total_blocks.div_ceil(8) as usize;
        let mut bits = vec![0u8; byte_count];
        let copy_len = data.len().min(byte_count);
        bits[..copy_len].copy_from_slice(&data[..copy_len]);

        // Byte-wise POPCNT count; mask the tail byte.
        let mut alloc_count: u64 = 0;
        let full_bytes = (total_blocks / 8) as usize;
        for byte in &bits[..full_bytes] {
            alloc_count += byte.count_ones() as u64;
        }
        let remaining_bits = (total_blocks % 8) as u32;
        if remaining_bits > 0 && full_bytes < bits.len() {
            let mask = (1u8 << remaining_bits) - 1;
            alloc_count += (bits[full_bytes] & mask).count_ones() as u64;
        }

        Self {
            bits,
            total_blocks,
            free_count: total_blocks - alloc_count,
            search_hint: 0,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    pub fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    pub fn free_count(&self) -> u64 {
        self.free_count
    }

    pub fn allocated_count(&self) -> u64 {
        self.total_blocks - self.free_count
    }

    pub fn is_allocated(&self, block: u64) -> bool {
        if block >= self.total_blocks {
            return false;
        }
        let byte_idx = (block / 8) as usize;
        let bit_idx = (block % 8) as u32;
        self.bits[byte_idx] & (1 << bit_idx) != 0
    }

    /// Allocate one block; index is relative to data region start.
    pub fn alloc_block(&mut self) -> Result<u64, HelixError> {
        if self.free_count == 0 {
            return Err(HelixError::NoSpace);
        }

        let start = self.search_hint;
        for offset in 0..self.total_blocks {
            let idx = (start + offset) % self.total_blocks;
            let byte_idx = (idx / 8) as usize;
            let bit_idx = (idx % 8) as u32;
            if self.bits[byte_idx] & (1 << bit_idx) == 0 {
                self.bits[byte_idx] |= 1 << bit_idx;
                self.free_count -= 1;
                self.search_hint = (idx + 1) % self.total_blocks;
                return Ok(idx);
            }
        }

        Err(HelixError::NoSpace)
    }

    /// Allocate `count` contiguous blocks; returns starting index.
    pub fn alloc_contiguous(&mut self, count: u64) -> Result<u64, HelixError> {
        if count == 0 {
            return Err(HelixError::InvalidBlockSize);
        }
        if self.free_count < count {
            return Err(HelixError::NoSpace);
        }

        let mut run_start: u64 = 0;
        let mut run_len: u64 = 0;

        for idx in 0..self.total_blocks {
            if !self.is_allocated(idx) {
                if run_len == 0 {
                    run_start = idx;
                }
                run_len += 1;
                if run_len >= count {
                    for b in run_start..run_start + count {
                        let byte_idx = (b / 8) as usize;
                        let bit_idx = (b % 8) as u32;
                        self.bits[byte_idx] |= 1 << bit_idx;
                    }
                    self.free_count -= count;
                    self.search_hint = (run_start + count) % self.total_blocks;
                    return Ok(run_start);
                }
            } else {
                run_len = 0;
            }
        }

        Err(HelixError::NoSpace)
    }

    /// Idempotent mark. Used by bitmap rebuild after log replay.
    pub fn mark_block_used(&mut self, block: u64) {
        if block >= self.total_blocks {
            return;
        }
        let byte_idx = (block / 8) as usize;
        let bit_idx = (block % 8) as u32;
        if self.bits[byte_idx] & (1 << bit_idx) == 0 {
            self.bits[byte_idx] |= 1 << bit_idx;
            self.free_count -= 1;
        }
    }

    pub fn mark_range_used(&mut self, start: u64, count: u64) {
        for i in 0..count {
            self.mark_block_used(start + i);
        }
    }

    /// Free one block; double-free returns `BitmapCorrupt`.
    pub fn free_block(&mut self, block: u64) -> Result<(), HelixError> {
        if block >= self.total_blocks {
            return Err(HelixError::BitmapCorrupt);
        }
        let byte_idx = (block / 8) as usize;
        let bit_idx = (block % 8) as u32;
        if self.bits[byte_idx] & (1 << bit_idx) == 0 {
            return Err(HelixError::BitmapCorrupt);
        }
        self.bits[byte_idx] &= !(1 << bit_idx);
        self.free_count += 1;
        if block < self.search_hint {
            self.search_hint = block;
        }
        Ok(())
    }

    pub fn free_range(&mut self, start: u64, count: u64) -> Result<(), HelixError> {
        for i in 0..count {
            self.free_block(start + i)?;
        }
        Ok(())
    }

    /// Bitmap blocks needed on disk to cover `total_data_blocks`.
    pub fn disk_blocks_needed(total_data_blocks: u64) -> u64 {
        let bits_per_block = BLOCK_SIZE as u64 * 8;
        total_data_blocks.div_ceil(bits_per_block)
    }
}
