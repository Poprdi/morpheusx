//! Block bitmap allocator for HelixFS.
//!
//! Simple bitmap: 1 bit per 4 KiB data block.  Bit 0 = block is free,
//! bit 1 = block is allocated.  The bitmap occupies contiguous blocks
//! starting at `superblock.bitmap_start`.
//!
//! ## Layout
//!
//! One bitmap block covers 4096 × 8 = 32768 data blocks = 128 MiB.
//! A 1 TB partition has ~256 M data blocks → 7813 bitmap blocks ≈ 30 MiB.

use crate::error::HelixError;
use crate::types::BLOCK_SIZE;
use alloc::vec;
use alloc::vec::Vec;

/// In-memory block bitmap.
pub struct BlockBitmap {
    /// Raw bitmap data (1 bit per data block).
    bits: Vec<u8>,
    /// Total data blocks covered.
    total_blocks: u64,
    /// Cached count of free blocks.
    free_count: u64,
    /// Hint: block index to start searching from (speeds up sequential alloc).
    search_hint: u64,
}

impl BlockBitmap {
    /// Create a new bitmap for `total_blocks` data blocks, all initially free.
    pub fn new(total_blocks: u64) -> Self {
        let byte_count = ((total_blocks + 7) / 8) as usize;
        Self {
            bits: vec![0u8; byte_count],
            total_blocks,
            free_count: total_blocks,
            search_hint: 0,
        }
    }

    /// Load bitmap from raw bytes (read from disk).
    pub fn from_bytes(data: &[u8], total_blocks: u64) -> Self {
        let byte_count = ((total_blocks + 7) / 8) as usize;
        let mut bits = vec![0u8; byte_count];
        let copy_len = data.len().min(byte_count);
        bits[..copy_len].copy_from_slice(&data[..copy_len]);

        // Count allocated bits
        let mut alloc_count: u64 = 0;
        for i in 0..total_blocks {
            let byte_idx = (i / 8) as usize;
            let bit_idx = (i % 8) as u32;
            if bits[byte_idx] & (1 << bit_idx) != 0 {
                alloc_count += 1;
            }
        }

        Self {
            bits,
            total_blocks,
            free_count: total_blocks - alloc_count,
            search_hint: 0,
        }
    }

    /// Get the raw bitmap bytes for writing to disk.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// Total data blocks managed by this bitmap.
    pub fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    /// Number of free blocks.
    pub fn free_count(&self) -> u64 {
        self.free_count
    }

    /// Number of allocated blocks.
    pub fn allocated_count(&self) -> u64 {
        self.total_blocks - self.free_count
    }

    /// Is a specific block allocated?
    pub fn is_allocated(&self, block: u64) -> bool {
        if block >= self.total_blocks {
            return false;
        }
        let byte_idx = (block / 8) as usize;
        let bit_idx = (block % 8) as u32;
        self.bits[byte_idx] & (1 << bit_idx) != 0
    }

    /// Allocate a single block.  Returns the block index relative to the
    /// data region start.
    pub fn alloc_block(&mut self) -> Result<u64, HelixError> {
        if self.free_count == 0 {
            return Err(HelixError::NoSpace);
        }

        // Search from hint forward, wrapping around.
        let start = self.search_hint;
        for offset in 0..self.total_blocks {
            let idx = (start + offset) % self.total_blocks;
            let byte_idx = (idx / 8) as usize;
            let bit_idx = (idx % 8) as u32;
            if self.bits[byte_idx] & (1 << bit_idx) == 0 {
                // Found a free block — mark it.
                self.bits[byte_idx] |= 1 << bit_idx;
                self.free_count -= 1;
                self.search_hint = (idx + 1) % self.total_blocks;
                return Ok(idx);
            }
        }

        Err(HelixError::NoSpace)
    }

    /// Allocate `count` contiguous blocks.  Returns the starting block index.
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
                    // Found a contiguous run — mark all.
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

    /// Free a single block.
    pub fn free_block(&mut self, block: u64) -> Result<(), HelixError> {
        if block >= self.total_blocks {
            return Err(HelixError::BitmapCorrupt);
        }
        let byte_idx = (block / 8) as usize;
        let bit_idx = (block % 8) as u32;
        if self.bits[byte_idx] & (1 << bit_idx) == 0 {
            // Double-free — already free.
            return Err(HelixError::BitmapCorrupt);
        }
        self.bits[byte_idx] &= !(1 << bit_idx);
        self.free_count += 1;
        // Move hint back to help reuse.
        if block < self.search_hint {
            self.search_hint = block;
        }
        Ok(())
    }

    /// Free a contiguous range of blocks.
    pub fn free_range(&mut self, start: u64, count: u64) -> Result<(), HelixError> {
        for i in 0..count {
            self.free_block(start + i)?;
        }
        Ok(())
    }

    /// Number of bitmap blocks needed on disk.
    pub fn disk_blocks_needed(total_data_blocks: u64) -> u64 {
        let bits_per_block = BLOCK_SIZE as u64 * 8;
        (total_data_blocks + bits_per_block - 1) / bits_per_block
    }
}
