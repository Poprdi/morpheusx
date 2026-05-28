//! Create/overwrite files.

use crate::bitmap::BlockBitmap;
use crate::crc::{crc64, fnv1a_64};
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
use crate::types::*;
use alloc::vec;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

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
    if path.is_empty() || !path.starts_with('/') || path.len() > MAX_PATH_LEN {
        return Err(HelixError::PathInvalid);
    }

    ensure_parent_dirs(log, index, path, timestamp_ns)?;

    let path_hash = fnv1a_64(path.as_bytes());
    let content_crc = if data.is_empty() { 0 } else { crc64(data) };

    if data.len() <= INLINE_DATA_SIZE {
        // v2 payload: [path_len: u16][path][data].
        let path_b = path.as_bytes();
        let mut payload = Vec::with_capacity(2 + path_b.len() + data.len());
        payload.extend_from_slice(&(path_b.len() as u16).to_le_bytes());
        payload.extend_from_slice(path_b);
        payload.extend_from_slice(data);
        let lsn = log.append(LogOp::Write, path_hash, &payload, timestamp_ns)?;

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

    // Roll back the bitmap on any I/O failure to avoid orphaned blocks.
    let scale = BLOCK_SIZE as u64 / device_block_size as u64;
    let mut write_offset = 0usize;
    let mut io_failed = false;
    for i in 0..blocks_needed {
        let mut block_buf = vec![0u8; BLOCK_SIZE as usize];
        let chunk = (data.len() - write_offset).min(BLOCK_SIZE as usize);
        block_buf[..chunk].copy_from_slice(&data[write_offset..write_offset + chunk]);
        write_offset += chunk;

        let abs_block = data_start_block + data_start_relative + i;
        let lba = Lba(partition_lba_start + abs_block * scale);
        if block_io.write_blocks(lba, &block_buf).is_err() {
            io_failed = true;
            break;
        }
    }
    if io_failed {
        let _ = bitmap.free_range(data_start_relative, blocks_needed);
        return Err(HelixError::IoWriteFailed);
    }

    // Extent: [logical: u64][physical: u64][count: u32][pad: u32].
    let mut extent_payload = vec![0u8; 24];
    extent_payload[0..8].copy_from_slice(&0u64.to_le_bytes());
    extent_payload[8..16].copy_from_slice(&data_start_relative.to_le_bytes());
    extent_payload[16..20].copy_from_slice(&(blocks_needed as u32).to_le_bytes());

    // v3 extent: [path_len: u16][path][0xFF][file_size: u64 LE][extent...].
    // 0xFF marker distinguishes extent records from inline during replay.
    let path_b = path.as_bytes();
    let file_size_bytes = (data.len() as u64).to_le_bytes();
    let mut full_payload = Vec::with_capacity(2 + path_b.len() + 1 + 8 + extent_payload.len());
    full_payload.extend_from_slice(&(path_b.len() as u16).to_le_bytes());
    full_payload.extend_from_slice(path_b);
    full_payload.push(0xFF);
    full_payload.extend_from_slice(&file_size_bytes);
    full_payload.extend_from_slice(&extent_payload);
    let lsn = match log.append(LogOp::Write, path_hash, &full_payload, timestamp_ns) {
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

    for _ in 0..blocks_needed {
        let phys = bitmap.alloc_block()?;
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

    // Roll back all extents on I/O failure to avoid orphaned blocks.
    let scale = BLOCK_SIZE as u64 / device_block_size as u64;
    let mut write_offset = 0usize;
    let mut io_failed = false;
    'write_loop: for (_, physical, count) in &extents {
        for j in 0..*count as u64 {
            let mut block_buf = vec![0u8; BLOCK_SIZE as usize];
            let chunk = (data.len() - write_offset).min(BLOCK_SIZE as usize);
            block_buf[..chunk].copy_from_slice(&data[write_offset..write_offset + chunk]);
            write_offset += chunk;

            let abs_block = data_start_block + physical + j;
            let lba = Lba(partition_lba_start + abs_block * scale);
            if block_io.write_blocks(lba, &block_buf).is_err() {
                io_failed = true;
                break 'write_loop;
            }
        }
    }
    if io_failed {
        for (_, physical, count) in &extents {
            let _ = bitmap.free_range(*physical, *count as u64);
        }
        return Err(HelixError::IoWriteFailed);
    }

    let mut extent_payload = Vec::with_capacity(extents.len() * 24);
    for (logical, physical, count) in &extents {
        extent_payload.extend_from_slice(&logical.to_le_bytes());
        extent_payload.extend_from_slice(&physical.to_le_bytes());
        extent_payload.extend_from_slice(&count.to_le_bytes());
        extent_payload.extend_from_slice(&0u32.to_le_bytes());
    }

    // v3 extent: [path_len: u16][path][0xFF][file_size: u64 LE][extents...].
    let path_b = path.as_bytes();
    let file_size_bytes = (data.len() as u64).to_le_bytes();
    let mut full_payload = Vec::with_capacity(2 + path_b.len() + 1 + 8 + extent_payload.len());
    full_payload.extend_from_slice(&(path_b.len() as u16).to_le_bytes());
    full_payload.extend_from_slice(path_b);
    full_payload.push(0xFF);
    full_payload.extend_from_slice(&file_size_bytes);
    full_payload.extend_from_slice(&extent_payload);
    let lsn = match log.append(LogOp::Write, path_hash, &full_payload, timestamp_ns) {
        Ok(l) => l,
        Err(e) => {
            for (_, physical, count) in &extents {
                let _ = bitmap.free_range(*physical, *count as u64);
            }
            return Err(e);
        },
    };

    let first_block = extents.first().map(|e| e.1).unwrap_or(BLOCK_NULL);

    let is_update = index.lookup(path).is_some();
    let mut entry = NamespaceIndex::make_file_entry(
        path,
        lsn,
        data.len() as u64,
        timestamp_ns,
        None,
        first_block,
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
    Ok(lsn)
}

fn ensure_parent_dirs(
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
        current.push('/');

        if index.lookup(&current).is_none() {
            let hash = fnv1a_64(current.as_bytes());
            let dir_bytes = current.as_bytes();
            let mut dir_payload = Vec::with_capacity(2 + dir_bytes.len());
            dir_payload.extend_from_slice(&(dir_bytes.len() as u16).to_le_bytes());
            dir_payload.extend_from_slice(dir_bytes);
            let lsn = log.append(LogOp::MkDir, hash, &dir_payload, timestamp_ns)?;
            let entry = NamespaceIndex::make_dir_entry(&current, lsn, timestamp_ns);
            index.upsert(entry);
        }
    }

    Ok(())
}

pub fn delete_file(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    let _entry = index.lookup(path).ok_or(HelixError::NotFound)?;

    let path_hash = fnv1a_64(path.as_bytes());
    let del_bytes = path.as_bytes();
    let mut del_payload = Vec::with_capacity(2 + del_bytes.len());
    del_payload.extend_from_slice(&(del_bytes.len() as u16).to_le_bytes());
    del_payload.extend_from_slice(del_bytes);
    let lsn = log.append(LogOp::Delete, path_hash, &del_payload, timestamp_ns)?;

    index.mark_deleted(path)?;
    Ok(lsn)
}

pub fn rename(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    old_path: &str,
    new_path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    let entry = index.lookup(old_path).ok_or(HelixError::NotFound)?;
    let mut new_entry = *entry;

    let new_key = fnv1a_64(new_path.as_bytes());
    new_entry.key = new_key;
    let path_bytes = new_path.as_bytes();
    let len = path_bytes.len().min(MAX_PATH_LEN);
    new_entry.path = [0u8; 256];
    new_entry.path[..len].copy_from_slice(&path_bytes[..len]);
    new_entry.modified_ns = timestamp_ns;

    let old_hash = fnv1a_64(old_path.as_bytes());
    let old_bytes = old_path.as_bytes();
    let new_bytes = new_path.as_bytes();
    let mut ren_payload = Vec::with_capacity(2 + old_bytes.len() + 2 + new_bytes.len());
    ren_payload.extend_from_slice(&(old_bytes.len() as u16).to_le_bytes());
    ren_payload.extend_from_slice(old_bytes);
    ren_payload.extend_from_slice(&(new_bytes.len() as u16).to_le_bytes());
    ren_payload.extend_from_slice(new_bytes);
    let lsn = log.append_full(
        LogOp::Rename,
        old_hash,
        new_key,
        0,
        &ren_payload,
        timestamp_ns,
    )?;

    new_entry.lsn = lsn;

    index.mark_deleted(old_path)?;
    index.upsert(new_entry);

    Ok(lsn)
}
