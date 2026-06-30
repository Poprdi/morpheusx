//! On-disk extent node: a fragmented file's full run list in one leaf block,
//! `[ExtentNodeHeader(16)][ExtentEntry(24) × count]`, CRC32C over the block.
//! Lets read/unlink/bitmap-rebuild recover every block of a non-contiguous file
//! instead of assuming a single contiguous run from `extent_root`.

use crate::crc::crc32c;
use crate::error::HelixError;
use crate::types::*;
use alloc::vec;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

const HDR: usize = 16;
const CRC_OFF: usize = 8;
const EXTENT_LEAF: u8 = 1;

fn block_lba(partition_lba_start: u64, data_start_block: u64, dbs: u32, rel_block: u64) -> Lba {
    let scale = BLOCK_SIZE as u64 / dbs as u64;
    Lba(partition_lba_start + (data_start_block + rel_block) * scale)
}

/// Serialize `(logical, physical, count)` runs into the node block.
pub fn write_extent_node<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    node_block: u64,
    extents: &[(u64, u64, u32)],
) -> Result<(), HelixError> {
    if extents.is_empty() || extents.len() > EXTENTS_PER_LEAF {
        return Err(HelixError::ExtentCorrupt);
    }
    let mut buf = vec![0u8; BLOCK_SIZE as usize];
    buf[0] = EXTENT_LEAF;
    buf[4..8].copy_from_slice(&(extents.len() as u32).to_le_bytes());
    let mut off = HDR;
    for (logical, physical, count) in extents {
        buf[off..off + 8].copy_from_slice(&logical.to_le_bytes());
        buf[off + 8..off + 16].copy_from_slice(&physical.to_le_bytes());
        buf[off + 16..off + 20].copy_from_slice(&count.to_le_bytes());
        off += 24;
    }
    let crc = crc32c(&buf);
    buf[CRC_OFF..CRC_OFF + 4].copy_from_slice(&crc.to_le_bytes());

    let lba = block_lba(partition_lba_start, data_start_block, device_block_size, node_block);
    block_io
        .write_blocks(lba, &buf)
        .map_err(|_| HelixError::IoWriteFailed)?;
    Ok(())
}

/// Read and CRC-verify the node, returning its `(logical, physical, count)` runs.
pub fn read_extent_node<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    node_block: u64,
) -> Result<Vec<(u64, u64, u32)>, HelixError> {
    let lba = block_lba(partition_lba_start, data_start_block, device_block_size, node_block);
    let mut buf = vec![0u8; BLOCK_SIZE as usize];
    block_io
        .read_blocks(lba, &mut buf)
        .map_err(|_| HelixError::IoReadFailed)?;

    let count = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if count == 0 || count > EXTENTS_PER_LEAF {
        return Err(HelixError::ExtentCorrupt);
    }
    let stored = u32::from_le_bytes(buf[CRC_OFF..CRC_OFF + 4].try_into().unwrap());
    buf[CRC_OFF..CRC_OFF + 4].copy_from_slice(&[0u8; 4]);
    if crc32c(&buf) != stored {
        return Err(HelixError::ExtentCorrupt);
    }

    let mut extents = Vec::with_capacity(count);
    let mut off = HDR;
    for _ in 0..count {
        let logical = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        let physical = u64::from_le_bytes(buf[off + 8..off + 16].try_into().unwrap());
        let cnt = u32::from_le_bytes(buf[off + 16..off + 20].try_into().unwrap());
        extents.push((logical, physical, cnt));
        off += 24;
    }
    Ok(extents)
}

/// Free every block an extent file owns: its runs plus, when fragmented, the
/// node block. Best-effort — a double free from a corrupt node is ignored.
#[allow(clippy::too_many_arguments)]
pub fn free_file_blocks<B: BlockIo>(
    block_io: &mut B,
    bitmap: &mut crate::bitmap::BlockBitmap,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    extent_root: u64,
    size: u64,
    is_node: bool,
) {
    if extent_root == BLOCK_NULL {
        return;
    }
    if is_node {
        if let Ok(extents) = read_extent_node(
            block_io,
            partition_lba_start,
            data_start_block,
            device_block_size,
            extent_root,
        ) {
            for (_, physical, count) in extents {
                let _ = bitmap.free_range(physical, count as u64);
            }
        }
        let _ = bitmap.free_block(extent_root);
    } else {
        let blocks = size.div_ceil(BLOCK_SIZE as u64);
        let _ = bitmap.free_range(extent_root, blocks);
    }
}

/// Reassemble file contents by walking the node's runs into logical order.
pub fn read_extent_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    device_block_size: u32,
    node_block: u64,
    file_size: u64,
) -> Result<Vec<u8>, HelixError> {
    let extents = read_extent_node(
        block_io,
        partition_lba_start,
        data_start_block,
        device_block_size,
        node_block,
    )?;
    let scale = BLOCK_SIZE as u64 / device_block_size as u64;
    let mut result = vec![0u8; file_size as usize];

    for (logical, physical, count) in extents {
        for j in 0..count as u64 {
            let file_off = (logical + j) * BLOCK_SIZE as u64;
            if file_off >= file_size {
                break;
            }
            let lba = Lba(partition_lba_start + (data_start_block + physical + j) * scale);
            let mut blk = vec![0u8; BLOCK_SIZE as usize];
            block_io
                .read_blocks(lba, &mut blk)
                .map_err(|_| HelixError::IoReadFailed)?;
            let start = file_off as usize;
            let end = (start + BLOCK_SIZE as usize).min(file_size as usize);
            result[start..end].copy_from_slice(&blk[..end - start]);
        }
    }
    Ok(result)
}
