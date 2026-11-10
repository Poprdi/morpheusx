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
use alloc::vec::Vec;
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
    // SAFETY: buf is a freshly read BLOCK_SIZE-byte block and HelixSuperblock
    // is #[repr(C)] POD of exactly that size; read_unaligned avoids requiring
    // buf's (Vec<u8>, alignment 1) pointer to satisfy the struct's alignment.
    let sb_a: HelixSuperblock =
        unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const HelixSuperblock) };
    let a_valid = sb_a.is_valid();

    let lba_b = Lba(partition_lba_start + SUPERBLOCK_B_BLOCK * blocks_per_sector);
    block_io
        .read_blocks(lba_b, &mut buf)
        .map_err(|_| HelixError::IoReadFailed)?;
    // SAFETY: same as sb_a above — buf holds one freshly read BLOCK_SIZE block.
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

    // SAFETY: sb is #[repr(C)] and sized exactly BLOCK_SIZE (asserted in types.rs).
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

/// Replay tail..=head into `index`, collecting snapshot LSNs and the versions
/// they pin. Fails if the log does not reach `committed_lsn` intact.
pub fn replay_log<B: BlockIo>(
    block_io: &mut B,
    log: &LogEngine,
    index: &mut NamespaceIndex,
    snapshots: &mut Vec<Lsn>,
    retained: &mut Vec<IndexEntry>,
    checkpoint_lsn: Lsn,
    committed_lsn: Lsn,
) -> Result<Lsn, HelixError> {
    let start_offset = core::mem::size_of::<LogSegmentHeader>() as u32;
    let corrupt = HelixError::LogSegmentCorrupt;

    let highest_lsn = log.scan_forward(
        block_io,
        log.tail_segment(),
        start_offset,
        |hdr, payload| {
            // Records at or below the checkpoint are already in the loaded index.
            if hdr.lsn <= checkpoint_lsn {
                return Ok(());
            }
            let op = LogOp::from_u8(hdr.op).ok_or(corrupt)?;

            match op {
                LogOp::MkDir => {
                    let (path, _) = decode_path_payload(payload).ok_or(corrupt)?;
                    index.upsert(NamespaceIndex::make_dir_entry(
                        path,
                        hdr.lsn,
                        hdr.timestamp_ns,
                    ));
                },

                LogOp::Write => {
                    let (path, data) = decode_path_payload(payload).ok_or(corrupt)?;
                    // The IS_EXTENT header flag, never a payload byte, decides
                    // inline vs extent.
                    let mut entry = if hdr.flags & rec_flags::IS_EXTENT != 0 {
                        if data.len() < 17 {
                            return Err(corrupt);
                        }
                        let u64_at = |o: usize| {
                            let mut b = [0u8; 8];
                            b.copy_from_slice(&data[o..o + 8]);
                            u64::from_le_bytes(b)
                        };
                        let has_crc = data.len() >= 25;
                        let mut e = NamespaceIndex::make_file_entry(
                            path,
                            hdr.lsn,
                            u64_at(1),
                            hdr.timestamp_ns,
                            None,
                            u64_at(9),
                            if has_crc { u64_at(17) } else { 0 },
                        );
                        if has_crc {
                            e.flags |= entry_flags::HAS_CONTENT_CRC;
                        }
                        if data[0] == extent_kind::NODE {
                            e.flags |= entry_flags::IS_EXTENT_NODE;
                        }
                        e
                    } else {
                        if data.len() > INLINE_DATA_SIZE {
                            return Err(corrupt);
                        }
                        let crc = if data.is_empty() {
                            0
                        } else {
                            crate::crc::crc64(data)
                        };
                        NamespaceIndex::make_file_entry(
                            path,
                            hdr.lsn,
                            data.len() as u64,
                            hdr.timestamp_ns,
                            Some(data),
                            BLOCK_NULL,
                            crc,
                        )
                    };

                    // A rewrite keeps birth metadata and bumps the version
                    // count, same as the live write path.
                    if let Some(existing) = index.lookup(path) {
                        entry.created_ns = existing.created_ns;
                        entry.first_lsn = existing.first_lsn;
                        entry.version_count = existing.version_count + 1;
                        let old = *existing;
                        crate::retain::retire(snapshots, retained, &old, hdr.lsn);
                    }
                    index.upsert(entry);
                },

                LogOp::Delete => {
                    let (path, _) = decode_path_payload(payload).ok_or(corrupt)?;
                    if let Some(old) = index.lookup_flex(path).copied() {
                        crate::retain::retire(snapshots, retained, &old, hdr.lsn);
                    }
                    let _ = index.mark_deleted(path);
                },

                LogOp::Rename => {
                    // [old_path_len: u16][old_path][new_path_len: u16][new_path]
                    let (old_path, rest) = decode_path_payload(payload).ok_or(corrupt)?;
                    let (new_path, _) = decode_path_payload(rest).ok_or(corrupt)?;
                    if let Some(old_entry) = index.lookup(old_path).copied() {
                        if let Some(dest) = index.lookup(new_path).copied() {
                            crate::retain::retire(snapshots, retained, &dest, hdr.lsn);
                        }
                        let mut new_entry = old_entry;
                        new_entry.key = fnv1a_64(new_path.as_bytes());
                        new_entry.path = [0u8; 256];
                        new_entry.path[..new_path.len()].copy_from_slice(new_path.as_bytes());
                        new_entry.modified_ns = hdr.timestamp_ns;
                        let _ = index.mark_deleted(old_path);
                        index.upsert(new_entry);
                    }
                },

                LogOp::Snapshot => snapshots.push(hdr.lsn),

                // Never written by this engine.
                LogOp::SetMeta
                | LogOp::DedupRef
                | LogOp::TxBegin
                | LogOp::TxCommit
                | LogOp::TxAbort
                | LogOp::Checkpoint
                | LogOp::Truncate
                | LogOp::Append => {},
            }

            Ok(())
        },
    )?;

    // every record up to committed_lsn must have been seen
    let reached = if highest_lsn == 0 {
        committed_lsn <= checkpoint_lsn
    } else {
        highest_lsn == committed_lsn
    };
    if !reached {
        return Err(corrupt);
    }

    index.compact();

    Ok(highest_lsn)
}
