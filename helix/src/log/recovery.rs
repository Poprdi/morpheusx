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
use crate::crc::fnv1a_64;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
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

// ═══════════════════════════════════════════════════════════════════════
// LOG REPLAY — Rebuild in-memory index from log records
// ═══════════════════════════════════════════════════════════════════════

/// Decode the v2 path prefix from a log payload.
///
/// v2 format: `[path_len: u16 LE][path: path_len bytes][rest...]`
///
/// Returns `(path_str, rest_of_payload)` or `None` if malformed.
fn decode_path_payload(payload: &[u8]) -> Option<(&str, &[u8])> {
    if payload.len() < 2 {
        return None;
    }
    let path_len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    let data_start = 2 + path_len;
    if data_start > payload.len() || path_len == 0 || path_len > MAX_PATH_LEN {
        return None;
    }
    let path_bytes = &payload[2..2 + path_len];
    let path_str = core::str::from_utf8(path_bytes).ok()?;
    if !path_str.starts_with('/') {
        return None;
    }
    Some((path_str, &payload[data_start..]))
}

/// Replay the log from `tail_segment` through `head_segment` and rebuild
/// the in-memory [`NamespaceIndex`].
///
/// This is the core recovery path.  On mount:
///
/// 1. `recover_superblock()` picks the best superblock.
/// 2. `HelixInstance::new(sb)` creates the log engine + empty index.
/// 3. `log.reload_head_segment(bio)` loads the write buffer.
/// 4. **`replay_log(bio, log, index)`** scans all committed records and
///    rebuilds the index from the path-carrying payloads (v2 format).
///
/// Records that cannot be decoded (e.g. v1 payloads without paths) are
/// silently skipped.
///
/// # Safety
/// Caller must ensure `log` and `index` belong to the same `HelixInstance`.
pub fn replay_log<B: BlockIo>(
    block_io: &mut B,
    log: &LogEngine,
    index: &mut NamespaceIndex,
) -> Result<Lsn, HelixError> {
    // Clear any stale entries.
    index.clear();

    let start_offset = core::mem::size_of::<LogSegmentHeader>() as u32;

    let highest_lsn = log.scan_forward(
        block_io,
        log.tail_segment(),
        start_offset,
        |hdr, payload| {
            let op = match LogOp::from_u8(hdr.op) {
                Some(o) => o,
                None => return Ok(()), // skip unknown ops
            };

            match op {
                LogOp::MkDir => {
                    if let Some((path, _rest)) = decode_path_payload(payload) {
                        let entry = NamespaceIndex::make_dir_entry(path, hdr.lsn, hdr.timestamp_ns);
                        index.upsert(entry);
                    }
                }

                LogOp::Write => {
                    if let Some((path, data)) = decode_path_payload(payload) {
                        if data.len() <= INLINE_DATA_SIZE {
                            // Inline file
                            let crc = if data.is_empty() { 0 } else { crate::crc::crc64(data) };
                            let entry = NamespaceIndex::make_file_entry(
                                path, hdr.lsn, data.len() as u64,
                                hdr.timestamp_ns, Some(data), BLOCK_NULL, crc,
                            );
                            index.upsert(entry);
                        } else if data.len() >= 24 {
                            // Extent-based file — decode first extent for root
                            let mut phys_bytes = [0u8; 8];
                            phys_bytes.copy_from_slice(&data[8..16]);
                            let phys = u64::from_le_bytes(phys_bytes);
                            let mut count_bytes = [0u8; 4];
                            count_bytes.copy_from_slice(&data[16..20]);
                            let count = u32::from_le_bytes(count_bytes) as u64;
                            let file_size = count * BLOCK_SIZE as u64;
                            let crc = crate::crc::crc64(data);
                            let entry = NamespaceIndex::make_file_entry(
                                path, hdr.lsn, file_size,
                                hdr.timestamp_ns, None, phys, crc,
                            );
                            index.upsert(entry);
                        }
                    }
                }

                LogOp::Delete => {
                    if let Some((path, _rest)) = decode_path_payload(payload) {
                        let _ = index.mark_deleted(path);
                    }
                }

                LogOp::Rename => {
                    // v2 rename: [old_path_len: u16][old_path][new_path_len: u16][new_path]
                    if let Some((old_path, rest)) = decode_path_payload(payload) {
                        if rest.len() >= 2 {
                            let new_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
                            if rest.len() >= 2 + new_len {
                                if let Ok(new_path) = core::str::from_utf8(&rest[2..2 + new_len]) {
                                    // Copy old entry, update path/key, delete old
                                    if let Some(old_entry) = index.lookup(old_path) {
                                        let mut new_entry = *old_entry;
                                        new_entry.key = fnv1a_64(new_path.as_bytes());
                                        let nb = new_path.as_bytes();
                                        new_entry.path = [0u8; 256];
                                        let l = nb.len().min(MAX_PATH_LEN);
                                        new_entry.path[..l].copy_from_slice(&nb[..l]);
                                        new_entry.lsn = hdr.lsn;
                                        new_entry.modified_ns = hdr.timestamp_ns;
                                        let _ = index.mark_deleted(old_path);
                                        index.upsert(new_entry);
                                    }
                                }
                            }
                        }
                    }
                }

                // Transaction markers, snapshots, checkpoints — skip during replay
                LogOp::SetMeta | LogOp::DedupRef |
                LogOp::TxBegin | LogOp::TxCommit | LogOp::TxAbort |
                LogOp::Snapshot | LogOp::Checkpoint | LogOp::Truncate => {}

                LogOp::Append => {
                    // Append: extend an existing file's data.
                    // v2 payload: [path_len: u16][path][appended_data]
                    if let Some((path, appended)) = decode_path_payload(payload) {
                        if let Some(existing) = index.lookup_mut(path) {
                            let old_size = existing.size as usize;
                            let new_size = old_size + appended.len();

                            if existing.flags & entry_flags::IS_INLINE != 0 {
                                // Inline file — extend inline_data if it still fits.
                                if new_size <= INLINE_DATA_SIZE {
                                    existing.inline_data[old_size..new_size]
                                        .copy_from_slice(appended);
                                    existing.size = new_size as u64;
                                } else {
                                    // Overflow: data was promoted to an extent by the
                                    // write path. The extent metadata should appear
                                    // as a separate Write record; just update size.
                                    existing.flags &= !entry_flags::IS_INLINE;
                                    existing.size = new_size as u64;
                                }
                            } else {
                                // Extent-based file — data blocks were already
                                // written by the original append operation.
                                // Just update the size.
                                existing.size = new_size as u64;
                            }
                            existing.lsn = hdr.lsn;
                            existing.modified_ns = hdr.timestamp_ns;
                            existing.version_count += 1;
                        }
                        // If the entry doesn't exist yet, skip (orphaned append).
                    }
                }
            }

            Ok(())
        },
    )?;

    // Remove tombstoned entries after full replay.
    index.compact();

    Ok(highest_lsn)
}
