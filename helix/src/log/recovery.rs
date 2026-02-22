//! Log recovery — replay from last checkpoint to restore in-memory index.
//!
//! ## Algorithm
//!
//! 1. Read both superblocks.  Pick the one with the highest valid
//!    `committed_lsn` (or the only valid one).
//! 2. Load the B-tree root from `index_root_block` in that superblock.
//! 3. Scan the log forward from `checkpoint_lsn`, applying each valid
//!    record to the in-memory index.
//! 4. Stop at the first record with an invalid CRC (crash boundary).
//! 5. Update `committed_lsn` to the highest valid LSN seen.

use crate::error::HelixError;
use crate::types::*;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;
use alloc::vec;

/// Read and validate both superblocks, returning the best one.
pub fn recover_superblock<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    device_block_size: u32,
) -> Result<HelixSuperblock, HelixError> {
    let blocks_per_sector = BLOCK_SIZE as u64 / device_block_size as u64;

    let mut buf = vec![0u8; BLOCK_SIZE as usize];

    // Read superblock A (block 0).
    let lba_a = Lba(partition_lba_start + SUPERBLOCK_A_BLOCK * blocks_per_sector);
    block_io.read_blocks(lba_a, &mut buf).map_err(|_| HelixError::IoReadFailed)?;
    let sb_a: HelixSuperblock = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock)
    };
    let a_valid = sb_a.is_valid();

    // Read superblock B (block 1).
    let lba_b = Lba(partition_lba_start + SUPERBLOCK_B_BLOCK * blocks_per_sector);
    block_io.read_blocks(lba_b, &mut buf).map_err(|_| HelixError::IoReadFailed)?;
    let sb_b: HelixSuperblock = unsafe {
        core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock)
    };
    let b_valid = sb_b.is_valid();

    match (a_valid, b_valid) {
        (true, true) => {
            // Pick whichever has higher committed_lsn.
            if sb_b.committed_lsn > sb_a.committed_lsn {
                Ok(sb_b)
            } else {
                Ok(sb_a)
            }
        }
        (true, false) => Ok(sb_a),
        (false, true) => Ok(sb_b),
        (false, false) => Err(HelixError::NoValidSuperblock),
    }
}

/// Write a superblock to one of the two slots.
pub fn write_superblock<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    device_block_size: u32,
    sb: &mut HelixSuperblock,
    slot: u64, // 0 = A, 1 = B
) -> Result<(), HelixError> {
    sb.update_crc();

    let blocks_per_sector = BLOCK_SIZE as u64 / device_block_size as u64;
    let block = if slot == 0 {
        SUPERBLOCK_A_BLOCK
    } else {
        SUPERBLOCK_B_BLOCK
    };
    let lba = Lba(partition_lba_start + block * blocks_per_sector);

    let bytes = unsafe {
        core::slice::from_raw_parts(sb as *const _ as *const u8, BLOCK_SIZE as usize)
    };
    block_io.write_blocks(lba, bytes).map_err(|_| HelixError::IoWriteFailed)?;
    block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;

    Ok(())
}
