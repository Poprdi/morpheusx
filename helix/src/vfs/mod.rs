//! Virtual Filesystem layer — mount table, fd operations, /sys/ nodes.
//!
//! This module sits between the syscall interface and the Helix on-disk
//! implementation.  It provides:
//!
//! - **Mount table**: maps mount points to filesystem instances.
//! - **File descriptor operations**: open / read / write / seek / close.
//! - **`/sys/` virtual entries**: synthetic read-only nodes for system info.

pub mod global;

use crate::bitmap::BlockBitmap;
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::LogEngine;
use crate::ops;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;

// ═══════════════════════════════════════════════════════════════════════
// Helix instance — one per mounted HelixFS volume
// ═══════════════════════════════════════════════════════════════════════

/// A mounted HelixFS instance.
pub struct HelixInstance {
    /// The current superblock.
    pub sb: HelixSuperblock,
    /// In-memory log engine.
    pub log: LogEngine,
    /// In-memory namespace index.
    pub index: NamespaceIndex,
    /// In-memory block bitmap.
    pub bitmap: BlockBitmap,
    /// Partition start LBA on the device.
    pub partition_lba_start: u64,
    /// Device block size (bytes).
    pub device_block_size: u32,
}

impl HelixInstance {
    /// Create an instance from a superblock.
    pub fn new(
        sb: HelixSuperblock,
        partition_lba_start: u64,
        device_block_size: u32,
    ) -> Self {
        let log = LogEngine::new(&sb, partition_lba_start, device_block_size);
        let index = NamespaceIndex::new();
        let bitmap = BlockBitmap::new(sb.data_block_count);

        Self {
            sb,
            log,
            index,
            bitmap,
            partition_lba_start,
            device_block_size,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Mount table
// ═══════════════════════════════════════════════════════════════════════

/// Entry in the mount table.
pub struct MountEntry {
    /// Mount point path (e.g. "/" or "/data/").
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    /// The filesystem instance.
    pub fs: HelixInstance,
    /// Whether this mount is read-only.
    pub read_only: bool,
}

/// Global mount table.
pub struct MountTable {
    entries: [Option<MountEntry>; MAX_MOUNTS],
}

impl Default for MountTable {
    fn default() -> Self {
        Self::new()
    }
}

impl MountTable {
    pub const fn new() -> Self {
        // Can't use array::map in const context, use this pattern instead.
        Self {
            entries: [
                None, None, None, None, None, None, None, None,
                None, None, None, None, None, None, None, None,
            ],
        }
    }

    /// Mount a HelixFS instance at the given mount point.
    pub fn mount(
        &mut self,
        mount_point: &str,
        instance: HelixInstance,
        read_only: bool,
    ) -> Result<u8, HelixError> {
        for (idx, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                let mut mp = [0u8; 256];
                let bytes = mount_point.as_bytes();
                let len = bytes.len().min(255);
                mp[..len].copy_from_slice(&bytes[..len]);

                *slot = Some(MountEntry {
                    mount_point: mp,
                    mount_point_len: len as u16,
                    fs: instance,
                    read_only,
                });
                return Ok(idx as u8);
            }
        }
        Err(HelixError::MountTableFull)
    }

    /// Find the mount that handles a given path.
    pub fn resolve(&self, path: &str) -> Option<(u8, &MountEntry)> {
        let mut best: Option<(u8, &MountEntry, usize)> = None;

        for (idx, slot) in self.entries.iter().enumerate() {
            if let Some(entry) = slot {
                let mp = &entry.mount_point[..entry.mount_point_len as usize];
                if let Ok(mp_str) = core::str::from_utf8(mp) {
                    if path.starts_with(mp_str) {
                        let len = mp_str.len();
                        match &best {
                            Some((_, _, best_len)) if *best_len >= len => {}
                            _ => best = Some((idx as u8, entry, len)),
                        }
                    }
                }
            }
        }

        best.map(|(idx, entry, _)| (idx, entry))
    }

    /// Get a mutable reference to a mount entry by index.
    pub fn get_mut(&mut self, idx: u8) -> Option<&mut MountEntry> {
        self.entries.get_mut(idx as usize).and_then(|s| s.as_mut())
    }

    /// Get an immutable reference to a mount entry by index.
    pub fn get(&self, idx: u8) -> Option<&MountEntry> {
        self.entries.get(idx as usize).and_then(|s| s.as_ref())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// FD table (per-process)
// ═══════════════════════════════════════════════════════════════════════

/// Per-process file descriptor table.
#[derive(Clone, Copy)]
pub struct FdTable {
    pub fds: [FileDescriptor; MAX_FDS],
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FdTable {
    pub const fn new() -> Self {
        Self {
            fds: [FileDescriptor::empty(); MAX_FDS],
        }
    }

    /// Allocate the lowest available fd.
    pub fn alloc(&mut self) -> Result<usize, HelixError> {
        for (i, fd) in self.fds.iter().enumerate() {
            if !fd.is_open() {
                return Ok(i);
            }
        }
        Err(HelixError::TooManyOpenFiles)
    }

    /// Get a file descriptor by index.
    pub fn get(&self, fd: usize) -> Result<&FileDescriptor, HelixError> {
        if fd >= MAX_FDS {
            return Err(HelixError::InvalidFd);
        }
        let desc = &self.fds[fd];
        if !desc.is_open() {
            return Err(HelixError::InvalidFd);
        }
        Ok(desc)
    }

    /// Get a mutable file descriptor by index.
    pub fn get_mut(&mut self, fd: usize) -> Result<&mut FileDescriptor, HelixError> {
        if fd >= MAX_FDS {
            return Err(HelixError::InvalidFd);
        }
        let desc = &mut self.fds[fd];
        if !desc.is_open() {
            return Err(HelixError::InvalidFd);
        }
        Ok(desc)
    }

    /// Close a file descriptor.
    pub fn close(&mut self, fd: usize) -> Result<(), HelixError> {
        if fd >= MAX_FDS {
            return Err(HelixError::InvalidFd);
        }
        self.fds[fd] = FileDescriptor::empty();
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// High-level VFS operations (called by syscall handlers)
// ═══════════════════════════════════════════════════════════════════════

/// Open a file or directory.
///
/// Returns the file descriptor index.
pub fn vfs_open<B: BlockIo>(
    block_io: &mut B,
    mount_table: &mut MountTable,
    fd_table: &mut FdTable,
    path: &str,
    flags: u32,
    timestamp_ns: u64,
) -> Result<usize, HelixError> {
    // Resolve mount.
    let (mount_idx, _entry) = mount_table
        .resolve(path)
        .ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    // Enforce read-only.
    if entry.read_only
        && (flags & (open_flags::O_WRITE | open_flags::O_CREATE | open_flags::O_TRUNC) != 0)
    {
        return Err(HelixError::ReadOnly);
    }

    // Check if the file exists.
    let exists = entry.fs.index.lookup(path).is_some();

    if !exists && flags & open_flags::O_CREATE != 0 {
        // Create an empty file.
        ops::write::write_file(
            block_io,
            &mut entry.fs.log,
            &mut entry.fs.index,
            &mut entry.fs.bitmap,
            entry.fs.partition_lba_start,
            entry.fs.device_block_size,
            entry.fs.sb.data_start_block,
            path,
            &[],
            timestamp_ns,
        )?;
    } else if !exists {
        return Err(HelixError::NotFound);
    }

    // Look up the entry to get the key.
    let idx_entry = entry.fs.index.lookup(path).ok_or(HelixError::NotFound)?;
    let key = idx_entry.key;

    // Allocate fd.
    let fd_idx = fd_table.alloc()?;

    let mut fd_path = [0u8; 256];
    let path_bytes = path.as_bytes();
    let copy_len = path_bytes.len().min(255);
    fd_path[..copy_len].copy_from_slice(&path_bytes[..copy_len]);

    fd_table.fds[fd_idx] = FileDescriptor {
        key,
        path: fd_path,
        flags,
        offset: 0,
        mount_idx,
        _pad: [0; 3],
        pinned_lsn: 0,
    };

    // Handle O_AT_LSN: pinned_lsn would be set by the caller.
    // Handle O_TRUNC: truncate to 0.
    if flags & open_flags::O_TRUNC != 0 {
        ops::write::write_file(
            block_io,
            &mut entry.fs.log,
            &mut entry.fs.index,
            &mut entry.fs.bitmap,
            entry.fs.partition_lba_start,
            entry.fs.device_block_size,
            entry.fs.sb.data_start_block,
            path,
            &[],
            timestamp_ns,
        )?;
    }

    Ok(fd_idx)
}

/// Read from an open file descriptor.
pub fn vfs_read<B: BlockIo>(
    block_io: &mut B,
    mount_table: &MountTable,
    fd_table: &mut FdTable,
    fd: usize,
    buf: &mut [u8],
) -> Result<usize, HelixError> {
    let desc = fd_table.get(fd)?;
    if !desc.is_readable() {
        return Err(HelixError::PermissionDenied);
    }

    let mount_idx = desc.mount_idx;
    let fd_path = crate::index::btree::path_str(&desc.path);
    let offset = desc.offset;

    let entry = mount_table.get(mount_idx).ok_or(HelixError::MountNotFound)?;

    // Look up by full path to avoid hash-collision ambiguity.
    let idx_entry = entry.fs.index.lookup(fd_path).ok_or(HelixError::NotFound)?;

    // Read file data.
    let data = if idx_entry.flags & entry_flags::IS_INLINE != 0 {
        let size = idx_entry.size as usize;
        let mut v = alloc::vec![0u8; size];
        v.copy_from_slice(&idx_entry.inline_data[..size]);
        v
    } else {
        ops::read::read_file(
            // We need a mutable block_io but mount_table is immutable.
            // This means the caller must provide block_io separately.
            block_io,
            &entry.fs.index,
            entry.fs.partition_lba_start,
            entry.fs.sb.data_start_block,
            entry.fs.device_block_size,
            // We need to reconstruct the path from the index entry.
            crate::index::btree::path_str(&idx_entry.path),
        )?
    };

    // Copy from offset.
    let start = offset as usize;
    if start >= data.len() {
        return Ok(0); // EOF
    }
    let available = data.len() - start;
    let to_copy = available.min(buf.len());
    buf[..to_copy].copy_from_slice(&data[start..start + to_copy]);

    // Advance offset.
    let desc = fd_table.get_mut(fd)?;
    desc.offset += to_copy as u64;

    Ok(to_copy)
}

/// Write to an open file descriptor.
pub fn vfs_write<B: BlockIo>(
    block_io: &mut B,
    mount_table: &mut MountTable,
    fd_table: &mut FdTable,
    fd: usize,
    data: &[u8],
    timestamp_ns: u64,
) -> Result<usize, HelixError> {
    let desc = fd_table.get(fd)?;
    if !desc.is_writable() {
        return Err(HelixError::PermissionDenied);
    }

    let mount_idx = desc.mount_idx;
    let fd_path = String::from(crate::index::btree::path_str(&desc.path));

    let entry = mount_table.get_mut(mount_idx).ok_or(HelixError::MountNotFound)?;

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    // Write (overwrite) the file.
    ops::write::write_file(
        block_io,
        &mut entry.fs.log,
        &mut entry.fs.index,
        &mut entry.fs.bitmap,
        entry.fs.partition_lba_start,
        entry.fs.device_block_size,
        entry.fs.sb.data_start_block,
        &fd_path,
        data,
        timestamp_ns,
    )?;

    // Update offset.
    let desc = fd_table.get_mut(fd)?;
    desc.offset = data.len() as u64;

    Ok(data.len())
}

/// Seek within a file descriptor.
pub fn vfs_seek(
    mount_table: &MountTable,
    fd_table: &mut FdTable,
    fd: usize,
    offset: i64,
    whence: u64,
) -> Result<u64, HelixError> {
    let desc = fd_table.get(fd)?;
    let mount_idx = desc.mount_idx;
    let fd_path = crate::index::btree::path_str(&desc.path);

    let entry = mount_table.get(mount_idx).ok_or(HelixError::MountNotFound)?;
    let idx_entry = entry.fs.index.lookup(fd_path).ok_or(HelixError::NotFound)?;
    let file_size = idx_entry.size;

    let new_offset = match whence {
        SEEK_SET => {
            if offset < 0 {
                return Err(HelixError::InvalidOffset);
            }
            offset as u64
        }
        SEEK_CUR => {
            let cur = desc.offset as i64;
            let new = cur + offset;
            if new < 0 {
                return Err(HelixError::InvalidOffset);
            }
            new as u64
        }
        SEEK_END => {
            let end = file_size as i64;
            let new = end + offset;
            if new < 0 {
                return Err(HelixError::InvalidOffset);
            }
            new as u64
        }
        _ => return Err(HelixError::InvalidOffset),
    };

    let desc = fd_table.get_mut(fd)?;
    desc.offset = new_offset;
    Ok(new_offset)
}

/// Close a file descriptor.
pub fn vfs_close(fd_table: &mut FdTable, fd: usize) -> Result<(), HelixError> {
    fd_table.close(fd)
}

/// Stat a path.
pub fn vfs_stat(
    mount_table: &MountTable,
    path: &str,
) -> Result<FileStat, HelixError> {
    let (_mount_idx, entry) = mount_table
        .resolve(path)
        .ok_or(HelixError::MountNotFound)?;

    ops::read::stat_file(&entry.fs.index, path)
}

/// Read directory contents.
pub fn vfs_readdir(
    mount_table: &MountTable,
    path: &str,
) -> Result<Vec<DirEntry>, HelixError> {
    let (_mount_idx, entry) = mount_table
        .resolve(path)
        .ok_or(HelixError::MountNotFound)?;

    ops::dir::readdir(&entry.fs.index, path)
}

/// Create a directory.
pub fn vfs_mkdir(
    mount_table: &mut MountTable,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let (mount_idx, _entry) = mount_table
        .resolve(path)
        .ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    ops::dir::mkdir(&mut entry.fs.log, &mut entry.fs.index, path, timestamp_ns)?;
    Ok(())
}

/// Unlink (delete) a file or empty directory.
pub fn vfs_unlink(
    mount_table: &mut MountTable,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let (mount_idx, _entry) = mount_table
        .resolve(path)
        .ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    ops::dir::unlink(&mut entry.fs.log, &mut entry.fs.index, path, timestamp_ns)?;
    Ok(())
}

/// Rename a file or directory.
pub fn vfs_rename(
    mount_table: &mut MountTable,
    old_path: &str,
    new_path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let (mount_idx, _entry) = mount_table
        .resolve(old_path)
        .ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    ops::write::rename(
        &mut entry.fs.log,
        &mut entry.fs.index,
        old_path,
        new_path,
        timestamp_ns,
    )?;
    Ok(())
}

/// Flush all pending writes to disk and update the superblock.
pub fn vfs_sync<B: BlockIo>(
    block_io: &mut B,
    mount_table: &mut MountTable,
) -> Result<(), HelixError> {
    for entry in mount_table.entries.iter_mut().flatten() {
            // Flush the log.
            let committed_lsn = entry.fs.log.flush(block_io)?;

            // Update superblock fields (write_superblock calls update_crc).
            entry.fs.sb.committed_lsn = committed_lsn;
            entry.fs.sb.log_head_segment = entry.fs.log.head_segment();
            entry.fs.sb.log_head_offset = entry.fs.log.head_offset();
            entry.fs.sb.log_tail_segment = entry.fs.log.tail_segment();

            // Write both superblock slots.
            crate::log::recovery::write_superblock(
                block_io,
                entry.fs.partition_lba_start,
                entry.fs.device_block_size,
                &mut entry.fs.sb,
                0,
            )?;
            crate::log::recovery::write_superblock(
                block_io,
                entry.fs.partition_lba_start,
                entry.fs.device_block_size,
                &mut entry.fs.sb,
                1,
            )?;

            block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;
    }
    Ok(())
}
