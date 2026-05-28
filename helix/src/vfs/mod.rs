//! VFS layer: mount table, fd ops, `/sys/` virtual nodes. Sits between
//! syscalls and the on-disk Helix implementation.

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

pub struct HelixInstance {
    pub sb: HelixSuperblock,
    pub log: LogEngine,
    pub index: NamespaceIndex,
    pub bitmap: BlockBitmap,
    pub partition_lba_start: u64,
    pub device_block_size: u32,
}

impl HelixInstance {
    pub fn new(sb: HelixSuperblock, partition_lba_start: u64, device_block_size: u32) -> Self {
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

pub struct MountEntry {
    /// e.g. "/" or "/data/".
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    pub fs: HelixInstance,
    pub read_only: bool,
}

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
        // array::map isn't const.
        Self {
            entries: [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
        }
    }

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

    /// Longest matching prefix wins.
    pub fn resolve(&self, path: &str) -> Option<(u8, &MountEntry)> {
        let mut best: Option<(u8, &MountEntry, usize)> = None;

        for (idx, slot) in self.entries.iter().enumerate() {
            if let Some(entry) = slot {
                let mp = &entry.mount_point[..entry.mount_point_len as usize];
                if let Ok(mp_str) = core::str::from_utf8(mp) {
                    if path.starts_with(mp_str) {
                        let len = mp_str.len();
                        match &best {
                            Some((_, _, best_len)) if *best_len >= len => {},
                            _ => best = Some((idx as u8, entry, len)),
                        }
                    }
                }
            }
        }

        best.map(|(idx, entry, _)| (idx, entry))
    }

    pub fn get_mut(&mut self, idx: u8) -> Option<&mut MountEntry> {
        self.entries.get_mut(idx as usize).and_then(|s| s.as_mut())
    }

    pub fn get(&self, idx: u8) -> Option<&MountEntry> {
        self.entries.get(idx as usize).and_then(|s| s.as_ref())
    }
}

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

    /// Lowest free fd starting at 3 (0..=2 reserved for stdin/out/err).
    pub fn alloc(&mut self) -> Result<usize, HelixError> {
        for i in 3..MAX_FDS {
            if !self.fds[i].is_open() {
                return Ok(i);
            }
        }
        Err(HelixError::TooManyOpenFiles)
    }

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

    pub fn close(&mut self, fd: usize) -> Result<(), HelixError> {
        if fd >= MAX_FDS {
            return Err(HelixError::InvalidFd);
        }
        self.fds[fd] = FileDescriptor::empty();
        Ok(())
    }
}

pub fn vfs_open<B: BlockIo>(
    block_io: &mut B,
    mount_table: &mut MountTable,
    fd_table: &mut FdTable,
    path: &str,
    flags: u32,
    timestamp_ns: u64,
) -> Result<usize, HelixError> {
    let (mount_idx, _entry) = mount_table.resolve(path).ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only
        && (flags & (open_flags::O_WRITE | open_flags::O_CREATE | open_flags::O_TRUNC) != 0)
    {
        return Err(HelixError::ReadOnly);
    }

    let exists = entry.fs.index.lookup(path).is_some();

    if !exists && flags & open_flags::O_CREATE != 0 {
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

    let idx_entry = entry.fs.index.lookup(path).ok_or(HelixError::NotFound)?;
    let key = idx_entry.key;

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

    // O_AT_LSN: caller sets pinned_lsn. O_TRUNC: write empty.
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

    let entry = mount_table
        .get(mount_idx)
        .ok_or(HelixError::MountNotFound)?;

    // Look up by full path to dodge hash-collision ambiguity.
    let idx_entry = entry.fs.index.lookup(fd_path).ok_or(HelixError::NotFound)?;

    let data = if idx_entry.flags & entry_flags::IS_INLINE != 0 {
        let size = idx_entry.size as usize;
        let mut v = alloc::vec![0u8; size];
        v.copy_from_slice(&idx_entry.inline_data[..size]);
        v
    } else {
        ops::read::read_file(
            block_io,
            &entry.fs.index,
            entry.fs.partition_lba_start,
            entry.fs.sb.data_start_block,
            entry.fs.device_block_size,
            crate::index::btree::path_str(&idx_entry.path),
        )?
    };

    let start = offset as usize;
    if start >= data.len() {
        return Ok(0);
    }
    let available = data.len() - start;
    let to_copy = available.min(buf.len());
    buf[..to_copy].copy_from_slice(&data[start..start + to_copy]);

    let desc = fd_table.get_mut(fd)?;
    desc.offset += to_copy as u64;

    Ok(to_copy)
}

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

    let entry = mount_table
        .get_mut(mount_idx)
        .ok_or(HelixError::MountNotFound)?;

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

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

    let desc = fd_table.get_mut(fd)?;
    desc.offset = data.len() as u64;

    Ok(data.len())
}

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

    let entry = mount_table
        .get(mount_idx)
        .ok_or(HelixError::MountNotFound)?;
    let idx_entry = entry.fs.index.lookup(fd_path).ok_or(HelixError::NotFound)?;
    let file_size = idx_entry.size;

    let new_offset = match whence {
        SEEK_SET => {
            if offset < 0 {
                return Err(HelixError::InvalidOffset);
            }
            offset as u64
        },
        SEEK_CUR => {
            let cur = desc.offset as i64;
            let new = cur + offset;
            if new < 0 {
                return Err(HelixError::InvalidOffset);
            }
            new as u64
        },
        SEEK_END => {
            let end = file_size as i64;
            let new = end + offset;
            if new < 0 {
                return Err(HelixError::InvalidOffset);
            }
            new as u64
        },
        _ => return Err(HelixError::InvalidOffset),
    };

    let desc = fd_table.get_mut(fd)?;
    desc.offset = new_offset;
    Ok(new_offset)
}

pub fn vfs_close(fd_table: &mut FdTable, fd: usize) -> Result<(), HelixError> {
    fd_table.close(fd)
}

pub fn vfs_stat(mount_table: &MountTable, path: &str) -> Result<FileStat, HelixError> {
    let (_mount_idx, entry) = mount_table.resolve(path).ok_or(HelixError::MountNotFound)?;

    ops::read::stat_file(&entry.fs.index, path)
}

pub fn vfs_readdir(mount_table: &MountTable, path: &str) -> Result<Vec<DirEntry>, HelixError> {
    let (_mount_idx, entry) = mount_table.resolve(path).ok_or(HelixError::MountNotFound)?;

    ops::dir::readdir(&entry.fs.index, path)
}

pub fn vfs_mkdir(
    mount_table: &mut MountTable,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let (mount_idx, _entry) = mount_table.resolve(path).ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    ops::dir::mkdir(&mut entry.fs.log, &mut entry.fs.index, path, timestamp_ns)?;
    Ok(())
}

/// File or empty directory.
pub fn vfs_unlink(
    mount_table: &mut MountTable,
    path: &str,
    timestamp_ns: u64,
) -> Result<(), HelixError> {
    let (mount_idx, _entry) = mount_table.resolve(path).ok_or(HelixError::MountNotFound)?;

    let entry = mount_table.get_mut(mount_idx).unwrap();

    if entry.read_only {
        return Err(HelixError::ReadOnly);
    }

    ops::dir::unlink(
        &mut entry.fs.log,
        &mut entry.fs.index,
        &mut entry.fs.bitmap,
        path,
        timestamp_ns,
    )?;
    Ok(())
}

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

/// Flush log; update both superblock slots.
pub fn vfs_sync<B: BlockIo>(
    block_io: &mut B,
    mount_table: &mut MountTable,
) -> Result<(), HelixError> {
    for entry in mount_table.entries.iter_mut().flatten() {
        let committed_lsn = entry.fs.log.flush(block_io)?;

        // write_superblock calls update_crc.
        entry.fs.sb.committed_lsn = committed_lsn;
        entry.fs.sb.log_head_segment = entry.fs.log.head_segment();
        entry.fs.sb.log_head_offset = entry.fs.log.head_offset();
        entry.fs.sb.log_tail_segment = entry.fs.log.tail_segment();

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
