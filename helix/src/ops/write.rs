//! Create/overwrite files.

use crate::bitmap::BlockBitmap;
use crate::crc::{crc64, fnv1a_64};
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;

/// Extent Write payload:
/// `[path_len: u16][path][kind: u8][file_size: u64][extent_root: u64][crc64: u64]`.
fn build_extent_payload(
    path: &str,
    kind: u8,
    file_size: u64,
    extent_root: u64,
    content_crc: u64,
) -> Vec<u8> {
    let path_b = path.as_bytes();
    let mut p = Vec::with_capacity(2 + path_b.len() + 25);
    p.extend_from_slice(&(path_b.len() as u16).to_le_bytes());
    p.extend_from_slice(path_b);
    p.push(kind);
    p.extend_from_slice(&file_size.to_le_bytes());
    p.extend_from_slice(&extent_root.to_le_bytes());
    p.extend_from_slice(&content_crc.to_le_bytes());
    p
}

/// Inline if `data.len() <= INLINE_DATA_SIZE` (96 B); else allocate extent(s).
/// Auto-creates parent directories.
#[allow(clippy::too_many_arguments)]
pub fn write_file<B: BlockIo>(
    block_io: &mut B,
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    bitmap: &mut BlockBitmap,
    partition_lba_start: u64,
    device_block_size: u32,
    data_start_block: u64,
    path: &str,
    data: &[u8],
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    crate::index::btree::validate_path(path)?;
    if path.len() > 1 && path.ends_with('/') {
        return Err(HelixError::PathInvalid);
    }
    // A file write must never shadow or clobber an existing directory.
    if let Some(existing) = index.lookup_flex(path) {
        if existing.flags & entry_flags::IS_DIR != 0 {
            return Err(HelixError::IsADirectory);
        }
    }

    ensure_parent_dirs(block_io, log, index, path, timestamp_ns)?;

    let path_hash = fnv1a_64(path.as_bytes());
    let content_crc = if data.is_empty() { 0 } else { crc64(data) };

    if data.len() <= INLINE_DATA_SIZE {
        // v2 payload: [path_len: u16][path][data].
        let path_b = path.as_bytes();
        let mut payload = Vec::with_capacity(2 + path_b.len() + data.len());
        payload.extend_from_slice(&(path_b.len() as u16).to_le_bytes());
        payload.extend_from_slice(path_b);
        payload.extend_from_slice(data);
        let lsn = log.append(block_io, LogOp::Write, path_hash, &payload, timestamp_ns)?;

        let is_update = index.lookup(path).is_some();
        let mut entry = NamespaceIndex::make_file_entry(
            path,
            lsn,
            data.len() as u64,
            timestamp_ns,
            Some(data),
            BLOCK_NULL,
            content_crc,
        );

        if is_update {
            if let Some(existing) = index.lookup(path) {
                entry.created_ns = existing.created_ns;
                entry.first_lsn = existing.first_lsn;
                entry.version_count = existing.version_count + 1;
            }
        }

        index.upsert(entry);
        return Ok(lsn);
    }

    let blocks_needed = (data.len() as u64).div_ceil(BLOCK_SIZE as u64);

    // Prefer contiguous for sequential-read throughput; fall back to fragmented.
    let data_start_relative = match bitmap.alloc_contiguous(blocks_needed) {
        Ok(start) => start,
        Err(HelixError::NoSpace) => {
            return write_file_fragmented(
                block_io,
                log,
                index,
                bitmap,
                partition_lba_start,
                device_block_size,
                data_start_block,
                path,
                data,
                timestamp_ns,
                path_hash,
                content_crc,
            );
        },
        Err(e) => return Err(e),
    };

    if let Err(e) = crate::extent::write_run(
        block_io,
        partition_lba_start,
        data_start_block,
        device_block_size,
        data_start_relative,
        data,
    ) {
        let _ = bitmap.free_range(data_start_relative, blocks_needed);
        return Err(e);
    }

    let full_payload = build_extent_payload(
        path,
        extent_kind::CONTIGUOUS,
        data.len() as u64,
        data_start_relative,
        content_crc,
    );
    let lsn = match log.append_full(
        block_io,
        LogOp::Write,
        rec_flags::IS_EXTENT,
        path_hash,
        0,
        0,
        &full_payload,
        timestamp_ns,
    ) {
        Ok(l) => l,
        Err(e) => {
            let _ = bitmap.free_range(data_start_relative, blocks_needed);
            return Err(e);
        },
    };

    let is_update = index.lookup(path).is_some();
    let mut entry = NamespaceIndex::make_file_entry(
        path,
        lsn,
        data.len() as u64,
        timestamp_ns,
        None,
        data_start_relative,
        content_crc,
    );
    entry.flags |= entry_flags::HAS_CONTENT_CRC;

    if is_update {
        if let Some(existing) = index.lookup(path) {
            entry.created_ns = existing.created_ns;
            entry.first_lsn = existing.first_lsn;
            entry.version_count = existing.version_count + 1;
        }
    }

    index.upsert(entry);
    Ok(lsn)
}

#[allow(clippy::too_many_arguments)]
fn write_file_fragmented<B: BlockIo>(
    block_io: &mut B,
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    bitmap: &mut BlockBitmap,
    partition_lba_start: u64,
    device_block_size: u32,
    data_start_block: u64,
    path: &str,
    data: &[u8],
    timestamp_ns: u64,
    path_hash: u64,
    content_crc: u64,
) -> Result<Lsn, HelixError> {
    let blocks_needed = (data.len() as u64).div_ceil(BLOCK_SIZE as u64);

    // (logical, physical, count). Coalesce contiguous neighbors.
    let mut extents: Vec<(u64, u64, u32)> = Vec::new();
    let mut logical_block = 0u64;

    let release = |bitmap: &mut BlockBitmap, extents: &[(u64, u64, u32)]| {
        for (_, physical, count) in extents {
            let _ = bitmap.free_range(*physical, *count as u64);
        }
    };

    for _ in 0..blocks_needed {
        let phys = match bitmap.alloc_block() {
            Ok(p) => p,
            Err(e) => {
                release(bitmap, &extents);
                return Err(e);
            },
        };
        if let Some(last) = extents.last_mut() {
            if last.1 + last.2 as u64 == phys {
                last.2 += 1;
                logical_block += 1;
                continue;
            }
        }
        extents.push((logical_block, phys, 1));
        logical_block += 1;
    }
    // one leaf holds the whole run list; more runs than that cannot be stored
    if extents.len() > EXTENTS_PER_LEAF {
        release(bitmap, &extents);
        return Err(HelixError::NoSpace);
    }

    let mut written = 0usize;
    for (_, physical, count) in &extents {
        let len = (*count as usize * BLOCK_SIZE as usize).min(data.len() - written);
        if let Err(e) = crate::extent::write_run(
            block_io,
            partition_lba_start,
            data_start_block,
            device_block_size,
            *physical,
            &data[written..written + len],
        ) {
            release(bitmap, &extents);
            return Err(e);
        }
        written += len;
    }

    let free_extents = |bitmap: &mut BlockBitmap| {
        for (_, physical, count) in &extents {
            let _ = bitmap.free_range(*physical, *count as u64);
        }
    };

    // Persist the run list to a node block so reads, unlink, and bitmap rebuild
    // recover every (non-contiguous) block; one leaf caps the extent count.
    let node_block = match bitmap.alloc_block() {
        Ok(b) => b,
        Err(e) => {
            free_extents(bitmap);
            return Err(e);
        },
    };
    if let Err(e) = crate::extent::write_extent_node(
        block_io,
        partition_lba_start,
        data_start_block,
        device_block_size,
        node_block,
        &extents,
    ) {
        let _ = bitmap.free_block(node_block);
        free_extents(bitmap);
        return Err(e);
    }

    let full_payload = build_extent_payload(
        path,
        extent_kind::NODE,
        data.len() as u64,
        node_block,
        content_crc,
    );
    let lsn = match log.append_full(
        block_io,
        LogOp::Write,
        rec_flags::IS_EXTENT,
        path_hash,
        0,
        0,
        &full_payload,
        timestamp_ns,
    ) {
        Ok(l) => l,
        Err(e) => {
            let _ = bitmap.free_block(node_block);
            free_extents(bitmap);
            return Err(e);
        },
    };

    let is_update = index.lookup(path).is_some();
    let mut entry = NamespaceIndex::make_file_entry(
        path,
        lsn,
        data.len() as u64,
        timestamp_ns,
        None,
        node_block,
        content_crc,
    );
    entry.flags |= entry_flags::IS_EXTENT_NODE | entry_flags::HAS_CONTENT_CRC;

    if is_update {
        if let Some(existing) = index.lookup(path) {
            entry.created_ns = existing.created_ns;
            entry.first_lsn = existing.first_lsn;
            entry.version_count = existing.version_count + 1;
        }
    }

    index.upsert(entry);
    Ok(lsn)
}

fn ensure_parent_dirs<B: BlockIo>(
    block_io: &mut B,
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 1 {
        return Ok(());
    }

    let mut current = alloc::string::String::from("/");
    for part in &parts[..parts.len() - 1] {
        current.push_str(part);
        if index.lookup(&current).is_some() {
            return Err(HelixError::NotADirectory);
        }
        current.push('/');

        if index.lookup(&current).is_none() {
            let hash = fnv1a_64(current.as_bytes());
            let dir_bytes = current.as_bytes();
            let mut dir_payload = Vec::with_capacity(2 + dir_bytes.len());
            dir_payload.extend_from_slice(&(dir_bytes.len() as u16).to_le_bytes());
            dir_payload.extend_from_slice(dir_bytes);
            let lsn = log.append(block_io, LogOp::MkDir, hash, &dir_payload, timestamp_ns)?;
            let entry = NamespaceIndex::make_dir_entry(&current, lsn, timestamp_ns);
            index.upsert(entry);
        }
    }

    Ok(())
}

/// Log bytes a rename of `old_path` will append (upper bound).
pub fn rename_log_bytes(index: &NamespaceIndex, old_path: &str) -> u64 {
    let per_record = crate::log::MAX_RECORD_BYTES;
    match index.lookup_flex(old_path) {
        Some(e) if e.flags & entry_flags::IS_DIR != 0 => {
            let prefix = dir_prefix(old_path);
            let n = index
                .all_entries()
                .iter()
                .filter(|e| {
                    e.flags & entry_flags::IS_DELETED == 0
                        && crate::index::btree::path_str(&e.path).starts_with(prefix.as_str())
                })
                .count();
            n as u64 * per_record
        },
        _ => per_record,
    }
}

/// Returns the rename LSN and, for a file renamed over an existing file, the
/// entry it replaced (the caller releases its blocks).
pub fn rename<B: BlockIo>(
    block_io: &mut B,
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    old_path: &str,
    new_path: &str,
    timestamp_ns: u64,
) -> Result<(Lsn, Option<IndexEntry>), HelixError> {
    crate::index::btree::validate_path(new_path)?;

    let src = *index.lookup_flex(old_path).ok_or(HelixError::NotFound)?;

    if src.flags & entry_flags::IS_DIR != 0 {
        let old_prefix = dir_prefix(old_path);
        let new_prefix = dir_prefix(new_path);
        if new_prefix == old_prefix {
            return Ok((src.lsn, None));
        }
        if new_prefix.len() > MAX_PATH_LEN {
            return Err(HelixError::PathTooLong);
        }
        // Renaming a directory into its own subtree would loop forever.
        if new_prefix.starts_with(old_prefix.as_str()) {
            return Err(HelixError::PathInvalid);
        }
        if index.lookup_flex(&new_prefix).is_some() {
            return Err(HelixError::AlreadyExists);
        }

        ensure_parent_dirs(block_io, log, index, &new_prefix, timestamp_ns)?;

        let mut moves: Vec<(String, String)> = Vec::new();
        for entry in index.all_entries() {
            if entry.flags & entry_flags::IS_DELETED != 0 {
                continue;
            }
            let p = crate::index::btree::path_str(&entry.path);
            if p.starts_with(old_prefix.as_str()) {
                let mut to = new_prefix.clone();
                to.push_str(&p[old_prefix.len()..]);
                if to.len() > MAX_PATH_LEN {
                    return Err(HelixError::PathTooLong);
                }
                moves.push((String::from(p), to));
            }
        }

        let mut last = 0;
        for (from, to) in &moves {
            last = log_rename_one(block_io, log, index, from, to, timestamp_ns)?;
        }
        return Ok((last, None));
    }

    if new_path.ends_with('/') {
        return Err(HelixError::PathInvalid);
    }
    let src_path = crate::index::btree::path_str(&src.path);
    if src_path == new_path {
        return Ok((src.lsn, None));
    }

    // A file may replace a file; a directory target is refused.
    let replaced = match index.lookup_flex(new_path) {
        Some(dest) if dest.flags & entry_flags::IS_DIR != 0 => {
            return Err(HelixError::IsADirectory);
        },
        Some(dest) => Some(*dest),
        None => None,
    };

    ensure_parent_dirs(block_io, log, index, new_path, timestamp_ns)?;

    // upsert in log_rename_one overwrites the destination entry in place
    let lsn = log_rename_one(block_io, log, index, src_path, new_path, timestamp_ns)?;
    Ok((lsn, replaced))
}

fn dir_prefix(path: &str) -> String {
    let mut s = String::from(path);
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

fn log_rename_one<B: BlockIo>(
    block_io: &mut B,
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    old_full: &str,
    new_full: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    let mut new_entry = *index.lookup(old_full).ok_or(HelixError::NotFound)?;
    let new_key = fnv1a_64(new_full.as_bytes());
    new_entry.key = new_key;
    new_entry.path = [0u8; 256];
    let nb = new_full.as_bytes();
    let len = nb.len().min(MAX_PATH_LEN);
    new_entry.path[..len].copy_from_slice(&nb[..len]);
    new_entry.modified_ns = timestamp_ns;

    let old_hash = fnv1a_64(old_full.as_bytes());
    let old_bytes = old_full.as_bytes();
    let mut ren_payload = Vec::with_capacity(2 + old_bytes.len() + 2 + nb.len());
    ren_payload.extend_from_slice(&(old_bytes.len() as u16).to_le_bytes());
    ren_payload.extend_from_slice(old_bytes);
    ren_payload.extend_from_slice(&(nb.len() as u16).to_le_bytes());
    ren_payload.extend_from_slice(nb);
    let lsn = log.append_full(
        block_io,
        LogOp::Rename,
        0,
        old_hash,
        new_key,
        0,
        &ren_payload,
        timestamp_ns,
    )?;

    // entry.lsn stays the lsn of the data version; a rename moves no data
    crate::retain::set_path_lsn(&mut new_entry, lsn);
    index.mark_deleted(old_full)?;
    index.upsert(new_entry);
    Ok(lsn)
}
