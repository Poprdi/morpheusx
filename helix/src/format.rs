//! Partition layout:
//!   0..1  dual superblocks
//!   2..   log region (`LOG_SEGMENT_BLOCKS` × `log_segment_count`)
//!   then  bitmap, then data.

use crate::error::HelixError;
use crate::log::recovery::write_superblock;
use crate::types::*;
use alloc::vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Format a partition as HelixFS. `total_sectors` is in device sectors.
pub fn format_helix<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    total_sectors: u64,
    device_block_size: u32,
    label: &str,
    uuid: [u8; 16],
) -> Result<HelixSuperblock, HelixError> {
    let sector_scale = BLOCK_SIZE / device_block_size;
    let total_fs_blocks = total_sectors / sector_scale as u64;

    if total_fs_blocks < 64 {
        return Err(HelixError::FormatTooSmall);
    }

    let superblock_blocks: u64 = 2;

    // ~1% of disk for the log, 1..=64 segments.
    let desired_log_segments = (total_fs_blocks / (100 * LOG_SEGMENT_BLOCKS)).clamp(1, 64);
    let log_blocks = desired_log_segments * LOG_SEGMENT_BLOCKS;

    let log_start = superblock_blocks;
    let log_end = log_start + log_blocks - 1;

    // 32768 data blocks covered per bitmap block.
    let data_blocks_approx = total_fs_blocks - superblock_blocks - log_blocks;
    let bitmap_blocks = data_blocks_approx.div_ceil(32768);

    let bitmap_start = log_end + 1;
    let data_start = bitmap_start + bitmap_blocks;
    let data_block_count = total_fs_blocks.saturating_sub(data_start);

    if data_block_count == 0 {
        return Err(HelixError::FormatTooSmall);
    }

    let mut sb = HelixSuperblock::zeroed();
    sb.magic = HELIX_MAGIC;
    sb.version = HELIX_VERSION;
    sb.block_size = BLOCK_SIZE;
    sb.total_blocks = total_fs_blocks;
    sb.log_start_block = log_start;
    sb.log_end_block = log_end;
    sb.log_segment_count = desired_log_segments;
    sb.bitmap_start = bitmap_start;
    sb.bitmap_blocks = bitmap_blocks;
    sb.data_start_block = data_start;
    sb.data_block_count = data_block_count;
    sb.index_root_block = BLOCK_NULL;
    sb.index_depth = 0;
    sb.committed_lsn = 0;
    sb.checkpoint_lsn = 0;
    sb.log_head_segment = 0;
    sb.log_tail_segment = 0;
    sb.log_head_offset = core::mem::size_of::<LogSegmentHeader>() as u32;
    sb.uuid = uuid;

    let label_bytes = label.as_bytes();
    let label_len = label_bytes.len().min(63);
    sb.label[..label_len].copy_from_slice(&label_bytes[..label_len]);

    sb.snapshot_count = 0;
    sb.snapshot_table_block = BLOCK_NULL;
    sb.blocks_used = superblock_blocks + log_blocks + bitmap_blocks;
    sb.file_count = 0;
    sb.dir_count = 1; // root
    sb.created_ns = 0; // caller sets real timestamp
    sb.last_mount_ns = 0;
    sb.mount_count = 0;

    let seg_header = LogSegmentHeader {
        magic: LOG_SEGMENT_MAGIC,
        _pad_magic: 0,
        sequence: 0,
        lsn_start: 0,
        record_count: 0,
        bytes_used: 0,
        timestamp_ns: 0,
        crc32c: 0,
        _reserved: [0u8; 20],
    };

    let seg_block_lba = partition_lba_start + log_start * sector_scale as u64;
    let scale = (BLOCK_SIZE / device_block_size) as u64;
    let mut seg_buf = vec![0u8; BLOCK_SIZE as usize];
    let hdr_bytes = unsafe {
        core::slice::from_raw_parts(
            &seg_header as *const _ as *const u8,
            core::mem::size_of::<LogSegmentHeader>(),
        )
    };
    seg_buf[..hdr_bytes.len()].copy_from_slice(hdr_bytes);

    for j in 0..scale {
        let lba = Lba(seg_block_lba + j);
        let start = (j as usize) * device_block_size as usize;
        let end = start + device_block_size as usize;
        block_io
            .write_blocks(lba, &seg_buf[start..end])
            .map_err(|_| HelixError::IoWriteFailed)?;
    }

    let zero_block = vec![0u8; device_block_size as usize];
    for blk in 0..bitmap_blocks {
        let abs_lba = partition_lba_start + (bitmap_start + blk) * scale;
        for j in 0..scale {
            block_io
                .write_blocks(Lba(abs_lba + j), &zero_block)
                .map_err(|_| HelixError::IoWriteFailed)?;
        }
    }

    sb.update_crc();
    write_superblock(block_io, partition_lba_start, device_block_size, &mut sb, 0)?;
    write_superblock(block_io, partition_lba_start, device_block_size, &mut sb, 1)?;

    block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;

    Ok(sb)
}

/// Quick HelixFS check via superblock A only.
pub fn is_helix<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    device_block_size: u32,
) -> bool {
    let scale = (BLOCK_SIZE / device_block_size) as u64;
    let mut buf = vec![0u8; BLOCK_SIZE as usize];

    for j in 0..scale {
        let lba = Lba(partition_lba_start + j);
        let start = (j as usize) * device_block_size as usize;
        let end = start + device_block_size as usize;
        if block_io.read_blocks(lba, &mut buf[start..end]).is_err() {
            return false;
        }
    }

    // buf is a Vec<u8> (alignment 1); copy out rather than form a misaligned ref.
    let sb: HelixSuperblock =
        unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock) };
    sb.is_valid()
}
