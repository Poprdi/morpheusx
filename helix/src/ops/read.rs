//! Read operations — read files, temporal access, version listing.

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
use crate::types::*;
use alloc::vec;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Read the current (latest) version of a file.
///
/// Returns the file contents as a byte vector.
///
/// For inline files (≤ 96 bytes), data is returned from the index entry.
/// For large files, data is read from the extent-mapped data blocks.
pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    index: &NamespaceIndex,
    partition_lba_start: u64,
    data_region_start_block: u64,
    device_block_size: u32,
    path: &str,
) -> Result<Vec<u8>, HelixError> {
    let entry = index.lookup(path).ok_or(HelixError::NotFound)?;

    if entry.flags & entry_flags::IS_DIR != 0 {
        return Err(HelixError::IsADirectory);
    }

    if entry.flags & entry_flags::IS_DELETED != 0 {
        return Err(HelixError::NotFound);
    }

    // Inline file.
    if entry.flags & entry_flags::IS_INLINE != 0 {
        let size = entry.size as usize;
        let mut data = vec![0u8; size];
        data.copy_from_slice(&entry.inline_data[..size]);
        return Ok(data);
    }

    // Extent-based file — read from data blocks.
    read_extent_data(
        block_io,
        partition_lba_start,
        data_region_start_block,
        device_block_size,
        entry.extent_root,
        entry.size,
    )
}

/// Read a file as it existed at a specific LSN (time-travel read).
///
/// Scans the log from the beginning up to `target_lsn` and reconstructs
/// the file state as of that point.
pub fn read_file_at_lsn<B: BlockIo>(
    block_io: &mut B,
    log: &LogEngine,
    partition_lba_start: u64,
    data_region_start_block: u64,
    device_block_size: u32,
    path: &str,
    target_lsn: Lsn,
) -> Result<Vec<u8>, HelixError> {
    let path_hash = fnv1a_64(path.as_bytes());

    // Scan log from the beginning up to target_lsn to find the
    // most recent Write or Append for this path.
    let mut last_write_data: Option<Vec<u8>> = None;
    let mut last_extent_root: Option<u64> = None;
    let mut last_file_size: Option<u64> = None;

    // Walk only live log segments: tail → head (circular).
    let segment_count = log.segment_count();
    let tail = log.tail_segment();
    let head = log.head_segment();
    let head_offset = log.head_offset();

    let mut seg = tail;
    loop {
        let mut offset = core::mem::size_of::<LogSegmentHeader>() as u32;

        loop {
            // If we're in the head segment, stop at the head write offset.
            if seg == head && offset >= head_offset {
                break;
            }

            match log.read_record(block_io, seg, offset) {
                Ok((header, payload)) => {
                    // Stop once we pass the target LSN.
                    if header.lsn > target_lsn {
                        break;
                    }

                    // Only care about records for this path.
                    if header.path_hash == path_hash {
                        if let Some(op) = LogOp::from_u8(header.op) {
                            match op {
                                LogOp::Write => {
                                    // v2 format: [path_len: u16][path][data...]
                                    // Skip the path prefix to get actual file data.
                                    let data = if payload.len() >= 2 {
                                        let plen =
                                            u16::from_le_bytes([payload[0], payload[1]]) as usize;
                                        let start = 2 + plen;
                                        if start <= payload.len() {
                                            &payload[start..]
                                        } else {
                                            &payload[..] // fallback: treat entire payload as data
                                        }
                                    } else {
                                        &payload[..]
                                    };

                                    if data.len() <= INLINE_DATA_SIZE {
                                        last_write_data = Some(data.to_vec());
                                        last_extent_root = None;
                                        last_file_size = None;
                                    } else if data.len() >= 24 {
                                        // Extent metadata.
                                        last_write_data = None;
                                        let mut phys_bytes = [0u8; 8];
                                        phys_bytes.copy_from_slice(&data[8..16]);
                                        last_extent_root = Some(u64::from_le_bytes(phys_bytes));
                                        let mut count_bytes = [0u8; 4];
                                        count_bytes.copy_from_slice(&data[16..20]);
                                        let block_count = u32::from_le_bytes(count_bytes) as u64;
                                        last_file_size = Some(block_count * BLOCK_SIZE as u64);
                                    }
                                }
                                LogOp::Delete => {
                                    last_write_data = None;
                                    last_extent_root = None;
                                    last_file_size = None;
                                }
                                _ => {}
                            }
                        }
                    }

                    // Advance by total_size() which is already 8-byte aligned.
                    offset += header.total_size() as u32;
                }
                Err(_) => break,
            }
        }

        // Done if we've processed the head segment.
        if seg == head {
            break;
        }
        seg = (seg + 1) % segment_count;
    }

    // Return the reconstructed state.
    if let Some(data) = last_write_data {
        return Ok(data);
    }

    if let Some(extent_root) = last_extent_root {
        let size = last_file_size.unwrap_or(0);
        return read_extent_data(
            block_io,
            partition_lba_start,
            data_region_start_block,
            device_block_size,
            extent_root,
            size,
        );
    }

    Err(HelixError::NotFound)
}

/// List all versions (LSNs) of a file.
///
/// Returns a vector of (lsn, timestamp_ns, op) tuples.
pub fn list_versions<B: BlockIo>(
    block_io: &mut B,
    log: &LogEngine,
    path: &str,
) -> Result<Vec<(Lsn, u64, LogOp)>, HelixError> {
    let path_hash = fnv1a_64(path.as_bytes());
    let mut versions = Vec::new();

    // Walk only live log segments: tail → head (circular).
    let segment_count = log.segment_count();
    let tail = log.tail_segment();
    let head = log.head_segment();
    let head_offset = log.head_offset();

    let mut seg = tail;
    loop {
        let mut offset = core::mem::size_of::<LogSegmentHeader>() as u32;

        loop {
            // Stop at head write offset in the head segment.
            if seg == head && offset >= head_offset {
                break;
            }

            match log.read_record(block_io, seg, offset) {
                Ok((header, _payload)) => {
                    if header.path_hash == path_hash {
                        if let Some(op) = LogOp::from_u8(header.op) {
                            match op {
                                LogOp::Write
                                | LogOp::Append
                                | LogOp::Delete
                                | LogOp::Rename
                                | LogOp::Truncate
                                | LogOp::SetMeta => {
                                    versions.push((header.lsn, header.timestamp_ns, op));
                                }
                                _ => {}
                            }
                        }
                    }

                    // Advance by total_size() which is already 8-byte aligned.
                    offset += header.total_size() as u32;
                }
                Err(_) => break,
            }
        }

        if seg == head {
            break;
        }
        seg = (seg + 1) % segment_count;
    }

    if versions.is_empty() {
        return Err(HelixError::NotFound);
    }

    Ok(versions)
}

/// Get file metadata from the index.
pub fn stat_file(index: &NamespaceIndex, path: &str) -> Result<FileStat, HelixError> {
    // Use flexible lookup: directories are stored with trailing '/' but
    // user paths typically omit it.
    let entry = index.lookup_flex(path).ok_or(HelixError::NotFound)?;

    if entry.flags & entry_flags::IS_DELETED != 0 {
        return Err(HelixError::NotFound);
    }

    Ok(FileStat {
        key: entry.key,
        size: entry.size,
        is_dir: entry.flags & entry_flags::IS_DIR != 0,
        created_ns: entry.created_ns,
        modified_ns: entry.modified_ns,
        version_count: entry.version_count,
        lsn: entry.lsn,
        first_lsn: entry.first_lsn,
        flags: entry.flags,
    })
}

/// Read data from extent-mapped data blocks.
fn read_extent_data<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    data_region_start_block: u64,
    device_block_size: u32,
    extent_root: u64,
    file_size: u64,
) -> Result<Vec<u8>, HelixError> {
    if extent_root == BLOCK_NULL {
        return Err(HelixError::ExtentCorrupt);
    }

    let blocks_needed = file_size.div_ceil(BLOCK_SIZE as u64);
    let mut result = Vec::with_capacity(file_size as usize);

    // Scale factor: how many device blocks per FS block.
    let scale = BLOCK_SIZE as u64 / device_block_size as u64;

    // Read contiguous blocks starting from extent_root.
    // This handles the simple single-extent case.
    // A full implementation would walk an extent tree for fragmented files.
    for i in 0..blocks_needed {
        let abs_block = data_region_start_block + extent_root + i;
        let lba = Lba(partition_lba_start + abs_block * scale);

        let mut block_buf = vec![0u8; BLOCK_SIZE as usize];

        // Read in device-block-sized chunks.
        for j in 0..scale {
            let chunk_lba = Lba(lba.0 + j);
            let start = (j as usize) * device_block_size as usize;
            let end = start + device_block_size as usize;
            block_io
                .read_blocks(chunk_lba, &mut block_buf[start..end])
                .map_err(|_| HelixError::IoReadFailed)?;
        }

        let remaining = (file_size as usize) - result.len();
        let copy_len = remaining.min(BLOCK_SIZE as usize);
        result.extend_from_slice(&block_buf[..copy_len]);
    }

    Ok(result)
}
