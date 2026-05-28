//! mkdir, readdir, unlink.

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::index::btree::{self, NamespaceIndex};
use crate::log::LogEngine;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;

/// Creates parents implicitly. Trailing '/' is canonical and added if missing.
pub fn mkdir(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(HelixError::PathInvalid);
    }

    let normalized = if path.ends_with('/') {
        String::from(path)
    } else {
        let mut s = String::from(path);
        s.push('/');
        s
    };

    if normalized.len() > MAX_PATH_LEN {
        return Err(HelixError::PathInvalid);
    }

    if let Some(existing) = index.lookup(&normalized) {
        if existing.flags & entry_flags::IS_DIR != 0
            && existing.flags & entry_flags::IS_DELETED == 0
        {
            return Err(HelixError::AlreadyExists);
        }
    }

    ensure_parent_dirs(log, index, &normalized, timestamp_ns)?;

    let hash = fnv1a_64(normalized.as_bytes());
    let norm_bytes = normalized.as_bytes();
    let mut payload = Vec::with_capacity(2 + norm_bytes.len());
    payload.extend_from_slice(&(norm_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(norm_bytes);
    let lsn = log.append(LogOp::MkDir, hash, &payload, timestamp_ns)?;
    let entry = NamespaceIndex::make_dir_entry(&normalized, lsn, timestamp_ns);
    index.upsert(entry);

    Ok(lsn)
}

/// Returns direct children only; names, not full paths.
pub fn readdir(index: &NamespaceIndex, dir_path: &str) -> Result<Vec<DirEntry>, HelixError> {
    let normalized = if dir_path == "/" {
        String::from("/")
    } else if dir_path.ends_with('/') {
        String::from(dir_path)
    } else {
        let mut s = String::from(dir_path);
        s.push('/');
        s
    };

    if normalized != "/" {
        let dir_entry = index.lookup(&normalized).ok_or(HelixError::NotFound)?;
        if dir_entry.flags & entry_flags::IS_DIR == 0 {
            return Err(HelixError::NotADirectory);
        }
        if dir_entry.flags & entry_flags::IS_DELETED != 0 {
            return Err(HelixError::NotFound);
        }
    }

    let children = index.readdir(&normalized);

    let mut entries = Vec::with_capacity(children.len());
    for child in children {
        let name_str = btree::path_str(&child.path);

        let filename = if child.flags & entry_flags::IS_DIR != 0 {
            let relative = &name_str[normalized.len()..];
            let trimmed = relative.trim_end_matches('/');
            if trimmed.contains('/') {
                continue;
            }
            trimmed
        } else {
            btree::filename(name_str)
        };

        if filename.is_empty() {
            continue;
        }

        let mut dir_entry = DirEntry {
            name: [0u8; 256],
            name_len: 0,
            size: child.size,
            is_dir: child.flags & entry_flags::IS_DIR != 0,
            modified_ns: child.modified_ns,
            version_count: child.version_count,
        };

        let name_bytes = filename.as_bytes();
        let len = name_bytes.len().min(255);
        dir_entry.name[..len].copy_from_slice(&name_bytes[..len]);
        dir_entry.name_len = len as u16;

        entries.push(dir_entry);
    }

    Ok(entries)
}

/// Directories must be empty. Inline data and directories own no blocks;
/// fragmented files only free their first extent here, the rest is GC's job.
pub fn unlink(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    bitmap: &mut crate::bitmap::BlockBitmap,
    path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    // Capture before any &mut borrow of index. Path may lack trailing '/'.
    let (extent_root, size, is_inline, is_dir) = {
        let entry = index.lookup_flex(path).ok_or(HelixError::NotFound)?;
        if entry.flags & entry_flags::IS_DELETED != 0 {
            return Err(HelixError::NotFound);
        }
        (
            entry.extent_root,
            entry.size,
            entry.flags & entry_flags::IS_INLINE != 0,
            entry.flags & entry_flags::IS_DIR != 0,
        )
    };

    if is_dir {
        let normalized = if path.ends_with('/') {
            String::from(path)
        } else {
            let mut s = String::from(path);
            s.push('/');
            s
        };

        let children = index.readdir(&normalized);
        if !children.is_empty() {
            return Err(HelixError::DirectoryNotEmpty);
        }
    }

    let actual_path = if index.lookup(path).is_some() {
        String::from(path)
    } else if !path.ends_with('/') {
        let mut s = String::from(path);
        s.push('/');
        s
    } else {
        String::from(&path[..path.len() - 1])
    };
    let hash = fnv1a_64(actual_path.as_bytes());
    let del_bytes = actual_path.as_bytes();
    let mut payload = Vec::with_capacity(2 + del_bytes.len());
    payload.extend_from_slice(&(del_bytes.len() as u16).to_le_bytes());
    payload.extend_from_slice(del_bytes);
    let lsn = log.append(LogOp::Delete, hash, &payload, timestamp_ns)?;
    index.mark_deleted(&actual_path)?;

    // Inline + dirs own no blocks. Fragmented files: only first extent freed
    // here; GC reclaims the rest from the log.
    if !is_inline && !is_dir && extent_root != crate::types::BLOCK_NULL {
        let blocks = size.div_ceil(crate::types::BLOCK_SIZE as u64);
        let _ = bitmap.free_range(extent_root, blocks);
    }

    Ok(lsn)
}

/// Mirrors write.rs.
fn ensure_parent_dirs(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let path_trimmed = path.trim_end_matches('/');
    let parts: Vec<&str> = path_trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 1 {
        return Ok(());
    }

    let mut current = String::from("/");
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
