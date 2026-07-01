//! Flat on-disk index checkpoint: every live `IndexEntry`, CRC-tagged and packed
//! 8 per block in a contiguous region. Lets mount restore the namespace without
//! replaying the whole log, so superseded log segments can be recycled (the log
//! ring is finite — without this it bricks at `LogFull`).

use crate::crc::crc32c;
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::types::*;
use alloc::vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

pub const ENTRIES_PER_INDEX_BLOCK: usize = BLOCK_SIZE as usize / 512;
const ENTRY_SIZE: usize = 512;

fn region_lba(partition_lba_start: u64, data_start_block: u64, dbs: u32, rel_block: u64) -> Lba {
    let scale = BLOCK_SIZE as u64 / dbs as u64;
    Lba(partition_lba_start + (data_start_block + rel_block) * scale)
}

fn entry_crc(e: &IndexEntry) -> u32 {
    let mut copy = *e;
    copy.crc32c = 0;
    let bytes = unsafe { core::slice::from_raw_parts(&copy as *const _ as *const u8, ENTRY_SIZE) };
    crc32c(bytes)
}

/// Number of blocks a region of `entry_count` entries occupies (>= 1).
pub fn region_blocks(entry_count: usize) -> u64 {
    entry_count.div_ceil(ENTRIES_PER_INDEX_BLOCK).max(1) as u64
}

pub fn write_index_region<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    region_start: u64,
    entries: &[IndexEntry],
) -> Result<(), HelixError> {
    let blocks = region_blocks(entries.len());
    for b in 0..blocks {
        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        for slot in 0..ENTRIES_PER_INDEX_BLOCK {
            let idx = b as usize * ENTRIES_PER_INDEX_BLOCK + slot;
            if idx >= entries.len() {
                break;
            }
            let mut e = entries[idx];
            e.crc32c = entry_crc(&e);
            let bytes =
                unsafe { core::slice::from_raw_parts(&e as *const _ as *const u8, ENTRY_SIZE) };
            buf[slot * ENTRY_SIZE..(slot + 1) * ENTRY_SIZE].copy_from_slice(bytes);
        }
        let lba = region_lba(
            partition_lba_start,
            data_start_block,
            device_block_size,
            region_start + b,
        );
        block_io
            .write_blocks(lba, &buf)
            .map_err(|_| HelixError::IoWriteFailed)?;
    }
    Ok(())
}

pub fn load_index_region<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    region_start: u64,
    block_count: u64,
    entry_count: u64,
    index: &mut NamespaceIndex,
) -> Result<(), HelixError> {
    let mut loaded: u64 = 0;
    for b in 0..block_count {
        if loaded >= entry_count {
            break;
        }
        let lba = region_lba(
            partition_lba_start,
            data_start_block,
            device_block_size,
            region_start + b,
        );
        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        block_io
            .read_blocks(lba, &mut buf)
            .map_err(|_| HelixError::IoReadFailed)?;
        for slot in 0..ENTRIES_PER_INDEX_BLOCK {
            if loaded >= entry_count {
                break;
            }
            let off = slot * ENTRY_SIZE;
            let e: IndexEntry =
                unsafe { core::ptr::read_unaligned(buf[off..].as_ptr() as *const IndexEntry) };
            if entry_crc(&e) != e.crc32c {
                return Err(HelixError::IndexCrcMismatch);
            }
            index.upsert(e);
            loaded += 1;
        }
    }
    Ok(())
}
