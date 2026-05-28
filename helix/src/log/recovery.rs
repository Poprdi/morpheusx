//! Log recovery: replay from last checkpoint to rebuild the in-memory index.
//!
//! 1. Pick the superblock with the highest valid `committed_lsn`.
//! 2. Scan log forward from `checkpoint_lsn`, applying each valid record.
//! 3. Stop at first bad CRC (crash boundary).
//! 4. Set `committed_lsn` to the highest valid LSN seen.

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
use crate::types::*;
use alloc::vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// Read both superblocks; return the one with the higher valid `committed_lsn`.
pub fn recover_superblock<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    device_block_size: u32,
) -> Result<HelixSuperblock, HelixError> {
    let blocks_per_sector = BLOCK_SIZE as u64 / device_block_size as u64;

    let mut buf = vec![0u8; BLOCK_SIZE as usize];

    let lba_a = Lba(partition_lba_start + SUPERBLOCK_A_BLOCK * blocks_per_sector);
    block_io
        .read_blocks(lba_a, &mut buf)
        .map_err(|_| HelixError::IoReadFailed)?;
    let sb_a: HelixSuperblock =
        unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock) };
    let a_valid = sb_a.is_valid();

    let lba_b = Lba(partition_lba_start + SUPERBLOCK_B_BLOCK * blocks_per_sector);
    block_io
        .read_blocks(lba_b, &mut buf)
        .map_err(|_| HelixError::IoReadFailed)?;
    let sb_b: HelixSuperblock =
        unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock) };
    let b_valid = sb_b.is_valid();

    match (a_valid, b_valid) {
        (true, true) => {
            if sb_b.committed_lsn > sb_a.committed_lsn {
                Ok(sb_b)
            } else {
                Ok(sb_a)
            }
        },
        (true, false) => Ok(sb_a),
        (false, true) => Ok(sb_b),
        (false, false) => Err(HelixError::NoValidSuperblock),
    }
}

/// `slot`: 0 = A, 1 = B.
pub fn write_superblock<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    device_block_size: u32,
    sb: &mut HelixSuperblock,
    slot: u64,
) -> Result<(), HelixError> {
    sb.update_crc();

    let blocks_per_sector = BLOCK_SIZE as u64 / device_block_size as u64;
    let block = if slot == 0 {
        SUPERBLOCK_A_BLOCK
    } else {
        SUPERBLOCK_B_BLOCK
    };
    let lba = Lba(partition_lba_start + block * blocks_per_sector);

    let bytes =
        unsafe { core::slice::from_raw_parts(sb as *const _ as *const u8, BLOCK_SIZE as usize) };
    block_io
        .write_blocks(lba, bytes)
        .map_err(|_| HelixError::IoWriteFailed)?;
    block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;

    Ok(())
}

/// v2 payload prefix: `[path_len: u16 LE][path][rest...]`.
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

/// Replay tail..=head, rebuilding the in-memory [`NamespaceIndex`]. v1 records
/// without v2 path prefix are silently skipped. `log` + `index` must be from
/// the same `HelixInstance`.
pub fn replay_log<B: BlockIo>(
    block_io: &mut B,
    log: &LogEngine,
    index: &mut NamespaceIndex,
) -> Result<Lsn, HelixError> {
    index.clear();

    let start_offset = core::mem::size_of::<LogSegmentHeader>() as u32;

    let highest_lsn = log.scan_forward(
        block_io,
        log.tail_segment(),
        start_offset,
        |hdr, payload| {
            let op = match LogOp::from_u8(hdr.op) {
                Some(o) => o,
                None => return Ok(()),
            };

            match op {
                LogOp::MkDir => {
                    if let Some((path, _rest)) = decode_path_payload(payload) {
                        let entry = NamespaceIndex::make_dir_entry(path, hdr.lsn, hdr.timestamp_ns);
                        index.upsert(entry);
                    }
                },

                LogOp::Write => {
                    if let Some((path, data)) = decode_path_payload(payload) {
                        // v3 extent: [0xFF][file_size: u64][logical: u64][physical: u64][count: u32][pad: u32].
                        // Inline: raw data, no 0xFF prefix, len <= INLINE_DATA_SIZE.
                        if !data.is_empty() && data[0] == 0xFF && data.len() >= 1 + 8 + 24 {
                            let file_size =
                                u64::from_le_bytes(data[1..9].try_into().unwrap_or([0u8; 8]));
                            let phys = u64::from_le_bytes(
                                data[9 + 8..9 + 16].try_into().unwrap_or([0u8; 8]),
                            );
                            let crc = crate::crc::crc64(&data[1..]);
                            let entry = NamespaceIndex::make_file_entry(
                                path,
                                hdr.lsn,
                                file_size,
                                hdr.timestamp_ns,
                                None,
                                phys,
                                crc,
                            );
                            index.upsert(entry);
                        } else if data.len() <= INLINE_DATA_SIZE {
                            let crc = if data.is_empty() {
                                0
                            } else {
                                crate::crc::crc64(data)
                            };
                            let entry = NamespaceIndex::make_file_entry(
                                path,
                                hdr.lsn,
                                data.len() as u64,
                                hdr.timestamp_ns,
                                Some(data),
                                BLOCK_NULL,
                                crc,
                            );
                            index.upsert(entry);
                        }
                    }
                },

                LogOp::Delete => {
                    if let Some((path, _rest)) = decode_path_payload(payload) {
                        let _ = index.mark_deleted(path);
                    }
                },

                LogOp::Rename => {
                    // v2: [old_path_len: u16][old_path][new_path_len: u16][new_path].
                    if let Some((old_path, rest)) = decode_path_payload(payload) {
                        if rest.len() >= 2 {
                            let new_len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
                            if rest.len() >= 2 + new_len {
                                if let Ok(new_path) = core::str::from_utf8(&rest[2..2 + new_len]) {
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
                },

                // Skipped during replay.
                LogOp::SetMeta
                | LogOp::DedupRef
                | LogOp::TxBegin
                | LogOp::TxCommit
                | LogOp::TxAbort
                | LogOp::Snapshot
                | LogOp::Checkpoint
                | LogOp::Truncate => {},

                LogOp::Append => {
                    // v2 payload: [path_len: u16][path][appended_data].
                    if let Some((path, appended)) = decode_path_payload(payload) {
                        if let Some(existing) = index.lookup_mut(path) {
                            let old_size = existing.size as usize;
                            let new_size = old_size + appended.len();

                            if existing.flags & entry_flags::IS_INLINE != 0 {
                                if new_size <= INLINE_DATA_SIZE {
                                    existing.inline_data[old_size..new_size]
                                        .copy_from_slice(appended);
                                    existing.size = new_size as u64;
                                } else {
                                    // Promoted to extent by write path; that
                                    // appears as a separate Write record.
                                    existing.flags &= !entry_flags::IS_INLINE;
                                    existing.size = new_size as u64;
                                }
                            } else {
                                // Data blocks already written by original append.
                                existing.size = new_size as u64;
                            }
                            existing.lsn = hdr.lsn;
                            existing.modified_ns = hdr.timestamp_ns;
                            existing.version_count += 1;
                        }
                        // Orphaned append (entry missing): skip.
                    }
                },
            }

            Ok(())
        },
    )?;

    index.compact();

    Ok(highest_lsn)
}
