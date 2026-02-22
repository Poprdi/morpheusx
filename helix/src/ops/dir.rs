//! Directory operations — mkdir, readdir, unlink.

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::index::btree::{self, NamespaceIndex};
use crate::log::LogEngine;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;

/// Create a directory.
///
/// The path must end with '/'.  Parent directories are created automatically.
pub fn mkdir(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    // Validate.
    if path.is_empty() || !path.starts_with('/') {
        return Err(HelixError::PathInvalid);
    }

    // Normalize: ensure trailing slash.
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

    // Check if it already exists.
    if let Some(existing) = index.lookup(&normalized) {
        if existing.flags & entry_flags::IS_DIR != 0
            && existing.flags & entry_flags::IS_DELETED == 0
        {
            return Err(HelixError::AlreadyExists);
        }
    }

    // Ensure parent directories exist.
    ensure_parent_dirs(log, index, &normalized, timestamp_ns)?;

    // Create the directory.
    let hash = fnv1a_64(normalized.as_bytes());
    let lsn = log.append(LogOp::MkDir, hash, &[], timestamp_ns)?;
    let entry = NamespaceIndex::make_dir_entry(&normalized, lsn, timestamp_ns);
    index.upsert(entry);

    Ok(lsn)
}

/// Read directory contents.
///
/// Returns a vector of `DirEntry` structs.  Each entry includes
/// the filename (not full path), size, and directory flag.
pub fn readdir(
    index: &NamespaceIndex,
    dir_path: &str,
) -> Result<Vec<DirEntry>, HelixError> {
    // Normalize path.
    let normalized = if dir_path == "/" {
        String::from("/")
    } else if dir_path.ends_with('/') {
        String::from(dir_path)
    } else {
        let mut s = String::from(dir_path);
        s.push('/');
        s
    };

    // Verify directory exists (root is always valid).
    if normalized != "/" {
        let dir_entry = index.lookup(&normalized).ok_or(HelixError::NotFound)?;
        if dir_entry.flags & entry_flags::IS_DIR == 0 {
            return Err(HelixError::NotADirectory);
        }
        if dir_entry.flags & entry_flags::IS_DELETED != 0 {
            return Err(HelixError::NotFound);
        }
    }

    // Prefix scan for children.
    let children = index.readdir(&normalized);

    let mut entries = Vec::with_capacity(children.len());
    for child in children {
        let name_str = btree::path_str(&child.path);

        // Extract just the filename from the full path.
        let filename = if child.flags & entry_flags::IS_DIR != 0 {
            // Directory: strip the normalized prefix and trailing slash.
            let relative = &name_str[normalized.len()..];
            let trimmed = relative.trim_end_matches('/');
            // Only direct children (no deeper nesting).
            if trimmed.contains('/') {
                continue;
            }
            trimmed
        } else {
            // File: extract filename.
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

/// Unlink (delete) a file or empty directory.
///
/// For directories, the directory must be empty.
pub fn unlink(
    log: &mut LogEngine,
    index: &mut NamespaceIndex,
    path: &str,
    timestamp_ns: u64,
) -> Result<Lsn, HelixError> {
    let entry = index.lookup(path).ok_or(HelixError::NotFound)?;

    if entry.flags & entry_flags::IS_DELETED != 0 {
        return Err(HelixError::NotFound);
    }

    // If this is a directory, verify it's empty.
    if entry.flags & entry_flags::IS_DIR != 0 {
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

    let hash = fnv1a_64(path.as_bytes());
    let lsn = log.append(LogOp::Delete, hash, &[], timestamp_ns)?;
    index.mark_deleted(path)?;

    Ok(lsn)
}

/// Ensure all parent directories exist (same logic as in write.rs).
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
            let lsn = log.append(LogOp::MkDir, hash, &[], timestamp_ns)?;
            let entry = NamespaceIndex::make_dir_entry(&current, lsn, timestamp_ns);
            index.upsert(entry);
        }
    }

    Ok(())
}
