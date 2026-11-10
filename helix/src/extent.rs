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

const IO_CHUNK_BLOCKS: usize = 16;

/// Read whole blocks starting at data block `physical` into `buf`.
pub fn read_run<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    dbs: u32,
    physical: u64,
    buf: &mut [u8],
) -> Result<(), HelixError> {
    let chunk_bytes = IO_CHUNK_BLOCKS * BLOCK_SIZE as usize;
    let mut block = physical;
    for chunk in buf.chunks_mut(chunk_bytes) {
        let lba = block_lba(partition_lba_start, data_start_block, dbs, block);
        block_io
            .read_blocks(lba, chunk)
            .map_err(|_| HelixError::IoReadFailed)?;
        block += (chunk.len() / BLOCK_SIZE as usize) as u64;
    }
    Ok(())
}

/// Write `data` starting at data block `physical`; a partial tail is zero-padded.
pub fn write_run<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_start_block: u64,
    dbs: u32,
    physical: u64,
    data: &[u8],
) -> Result<(), HelixError> {
    let bs = BLOCK_SIZE as usize;
    let full = data.len() / bs * bs;
    let mut block = physical;
    for chunk in data[..full].chunks(IO_CHUNK_BLOCKS * bs) {
        let lba = block_lba(partition_lba_start, data_start_block, dbs, block);
        block_io
            .write_blocks(lba, chunk)
            .map_err(|_| HelixError::IoWriteFailed)?;
        block += (chunk.len() / bs) as u64;
    }
    if full < data.len() {
        let mut tail = [0u8; BLOCK_SIZE as usize];
        tail[..data.len() - full].copy_from_slice(&data[full..]);
        let lba = block_lba(partition_lba_start, data_start_block, dbs, block);
        block_io
            .write_blocks(lba, &tail)
            .map_err(|_| HelixError::IoWriteFailed)?;
    }
    Ok(())
}

/// Zeroed buffer of `len` bytes, or `FileTooLarge` if the heap cannot hold it.
pub fn try_zeroed(len: usize) -> Result<Vec<u8>, HelixError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| HelixError::FileTooLarge)?;
    v.resize(len, 0);
    Ok(v)
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

    let lba = block_lba(
        partition_lba_start,
        data_start_block,
        device_block_size,
        node_block,
    );
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
    let lba = block_lba(
        partition_lba_start,
        data_start_block,
        device_block_size,
        node_block,
    );
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

/// Free a file's runs and node block. `defer` holds them until the next commit.
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
    defer: bool,
) {
    if extent_root == BLOCK_NULL {
        return;
    }
    let release = |bitmap: &mut crate::bitmap::BlockBitmap, start: u64, count: u64| {
        if defer {
            bitmap.defer_free_range(start, count);
        } else {
            let _ = bitmap.free_range(start, count);
        }
    };
    if is_node {
        if let Ok(extents) = read_extent_node(
            block_io,
            partition_lba_start,
            data_start_block,
            device_block_size,
            extent_root,
        ) {
            for (_, physical, count) in extents {
                release(bitmap, physical, count as u64);
            }
        }
        release(bitmap, extent_root, 1);
    } else {
        let blocks = size.div_ceil(BLOCK_SIZE as u64);
        release(bitmap, extent_root, blocks);
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
    // a file_size larger than its extents cover is a corrupt inode
    let mut covered: u64 = 0;
    for (logical, _, count) in &extents {
        if *logical != covered / BLOCK_SIZE as u64 {
            return Err(HelixError::ExtentCorrupt);
        }
        covered += *count as u64 * BLOCK_SIZE as u64;
    }
    if file_size > covered {
        return Err(HelixError::FileTooLarge);
    }
    let padded = file_size.div_ceil(BLOCK_SIZE as u64) * BLOCK_SIZE as u64;
    let mut result = try_zeroed(padded as usize)?;

    for (logical, physical, count) in extents {
        let start = logical * BLOCK_SIZE as u64;
        if start >= padded {
            break;
        }
        let end = (start + count as u64 * BLOCK_SIZE as u64).min(padded);
        read_run(
            block_io,
            partition_lba_start,
            data_start_block,
            device_block_size,
            physical,
            &mut result[start as usize..end as usize],
        )?;
    }
    result.truncate(file_size as usize);
    Ok(result)
}
