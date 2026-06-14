//! Pure HelixFS engine: inherent ops over any `B: BlockIo`. No mount table,
//! no fd table, no global state — those live in the kernel storage subsystem.
//! Construct via `mount`/`format_and_mount`; the caller owns the device and the
//! per-fd cursor state.

use crate::bitmap::BlockBitmap;
use crate::error::HelixError;
use crate::index::btree::NamespaceIndex;
use crate::log::recovery::{recover_superblock, replay_log, write_superblock};
use crate::log::LogEngine;
use crate::ops;
use crate::types::*;
use crate::{crc, format};
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;

/// One mounted HelixFS volume. Owns sb/log/index/bitmap and its partition LBA
/// base; all I/O goes through a borrowed `&mut B` so the device stays caller-owned.
pub struct HelixFs {
    pub sb: HelixSuperblock,
    pub log: LogEngine,
    pub index: NamespaceIndex,
    pub bitmap: BlockBitmap,
    pub partition_lba_start: u64,
    pub device_block_size: u32,
}

impl HelixFs {
    /// Assemble from an already-recovered superblock. Does not touch the device;
    /// the index is empty until `replay`. Prefer `mount` for the normal path.
    pub fn from_superblock(
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

    /// Format a fresh HelixFS over `[lba_start, lba_start+lba_count)` then mount it.
    pub fn format_and_mount<B: BlockIo>(
        block_io: &mut B,
        lba_start: u64,
        lba_count: u64,
        block_size: u32,
        label: &str,
        uuid: [u8; 16],
    ) -> Result<Self, HelixError> {
        format::format_helix(block_io, lba_start, lba_count, block_size, label, uuid)?;
        Self::mount(block_io, lba_start, block_size)
    }

    /// Recover the superblock, replay the log, rebuild the allocation bitmap.
    pub fn mount<B: BlockIo>(
        block_io: &mut B,
        lba_start: u64,
        block_size: u32,
    ) -> Result<Self, HelixError> {
        let sb = recover_superblock(block_io, lba_start, block_size)?;
        if sb.version != HELIX_VERSION {
            return Err(HelixError::IncompatibleVersion);
        }
        let mut fs = Self::from_superblock(sb, lba_start, block_size);
        // Reload head so flush() doesn't clobber existing records.
        fs.log.reload_head_segment(block_io)?;
        replay_log(block_io, &fs.log, &mut fs.index)?;
        fs.rebuild_bitmap_from_index();
        Ok(fs)
    }

    /// After replay the bitmap is zero; mark every extent-backed live file's
    /// blocks used or new allocations will overlap existing data.
    fn rebuild_bitmap_from_index(&mut self) {
        for entry in self.index.all_entries() {
            if entry.flags & entry_flags::IS_DELETED != 0 {
                continue;
            }
            if entry.flags & entry_flags::IS_DIR != 0 {
                continue;
            }
            if entry.flags & entry_flags::IS_INLINE != 0 {
                continue;
            }
            if entry.extent_root == BLOCK_NULL {
                continue;
            }
            let blocks_needed = entry.size.div_ceil(BLOCK_SIZE as u64);
            if blocks_needed > 0 {
                self.bitmap
                    .mark_range_used(entry.extent_root, blocks_needed);
            }
        }
    }

    /// Resolve `path` to its index key, creating an empty file if `O_CREATE` and
    /// absent, truncating if `O_TRUNC`. Returns the key the caller stores in its
    /// per-fd cookie. Read-only / permission policy is the caller's (it knows the
    /// mount's residency); the engine only enforces existence + create/trunc.
    pub fn open<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        flags: u32,
        timestamp_ns: u64,
    ) -> Result<u64, HelixError> {
        let exists = self.index.lookup(path).is_some();

        if !exists {
            if flags & open_flags::O_CREATE != 0 {
                self.write_empty(block_io, path, timestamp_ns)?;
            } else {
                return Err(HelixError::NotFound);
            }
        } else if flags & open_flags::O_TRUNC != 0 {
            self.write_empty(block_io, path, timestamp_ns)?;
        }

        let idx_entry = self.index.lookup(path).ok_or(HelixError::NotFound)?;
        Ok(idx_entry.key)
    }

    fn write_empty<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        ops::write::write_file(
            block_io,
            &mut self.log,
            &mut self.index,
            &mut self.bitmap,
            self.partition_lba_start,
            self.device_block_size,
            self.sb.data_start_block,
            path,
            &[],
            timestamp_ns,
        )?;
        Ok(())
    }

    /// Full file contents. The caller applies its own fd offset/slicing.
    pub fn read<B: BlockIo>(&self, block_io: &mut B, path: &str) -> Result<Vec<u8>, HelixError> {
        let idx_entry = self.index.lookup(path).ok_or(HelixError::NotFound)?;
        if idx_entry.flags & entry_flags::IS_INLINE != 0 {
            let size = idx_entry.size as usize;
            let mut v = alloc::vec![0u8; size];
            v.copy_from_slice(&idx_entry.inline_data[..size]);
            Ok(v)
        } else {
            ops::read::read_file(
                block_io,
                &self.index,
                self.partition_lba_start,
                self.sb.data_start_block,
                self.device_block_size,
                path,
            )
        }
    }

    /// Overwrite `path` with `data` (log-structured: a new version is appended).
    pub fn write<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        data: &[u8],
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        ops::write::write_file(
            block_io,
            &mut self.log,
            &mut self.index,
            &mut self.bitmap,
            self.partition_lba_start,
            self.device_block_size,
            self.sb.data_start_block,
            path,
            data,
            timestamp_ns,
        )?;
        Ok(())
    }

    pub fn stat(&self, path: &str) -> Result<FileStat, HelixError> {
        ops::read::stat_file(&self.index, path)
    }

    pub fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, HelixError> {
        ops::dir::readdir(&self.index, path)
    }

    pub fn mkdir(&mut self, path: &str, timestamp_ns: u64) -> Result<(), HelixError> {
        ops::dir::mkdir(&mut self.log, &mut self.index, path, timestamp_ns)?;
        Ok(())
    }

    /// File or empty directory.
    pub fn unlink(&mut self, path: &str, timestamp_ns: u64) -> Result<(), HelixError> {
        ops::dir::unlink(
            &mut self.log,
            &mut self.index,
            &mut self.bitmap,
            path,
            timestamp_ns,
        )?;
        Ok(())
    }

    pub fn rename(
        &mut self,
        old_path: &str,
        new_path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        ops::write::rename(
            &mut self.log,
            &mut self.index,
            old_path,
            new_path,
            timestamp_ns,
        )?;
        Ok(())
    }

    /// Resize `path`: shrink truncates, grow zero-extends. Read-modify-write that
    /// appends a new version, so old bytes stay recoverable via time-travel reads.
    /// Grows are capped (`MAX_TRUNCATE_GROW`) since the whole file is staged in
    /// the heap; an unbounded hostile grow would OOM the small kernel heap.
    pub fn truncate<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        new_size: u64,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        // Snapshot metadata, then drop the index borrow before the write borrow.
        let (is_inline, cur_size, inline_copy) = {
            let idx_entry = self.index.lookup(path).ok_or(HelixError::NotFound)?;
            if idx_entry.flags & entry_flags::IS_DIR != 0 {
                return Err(HelixError::IsADirectory);
            }
            if idx_entry.flags & entry_flags::IS_DELETED != 0 {
                return Err(HelixError::NotFound);
            }
            let is_inline = idx_entry.flags & entry_flags::IS_INLINE != 0;
            let inline_copy = if is_inline {
                let size = (idx_entry.size as usize).min(INLINE_DATA_SIZE);
                idx_entry.inline_data[..size].to_vec()
            } else {
                Vec::new()
            };
            (is_inline, idx_entry.size, inline_copy)
        };

        if new_size == cur_size {
            return Ok(());
        }
        if new_size > cur_size && new_size > MAX_TRUNCATE_GROW {
            return Err(HelixError::FileTooLarge);
        }

        let current = if is_inline {
            inline_copy
        } else {
            ops::read::read_file(
                block_io,
                &self.index,
                self.partition_lba_start,
                self.sb.data_start_block,
                self.device_block_size,
                path,
            )?
        };

        let mut new_data = current;
        new_data.resize(new_size as usize, 0u8);

        self.write(block_io, path, &new_data, timestamp_ns)
    }

    /// Every logged version of `path`, oldest-to-newest: (lsn, timestamp_ns, op).
    pub fn versions<B: BlockIo>(
        &self,
        block_io: &mut B,
        path: &str,
    ) -> Result<Vec<(Lsn, u64, LogOp)>, HelixError> {
        ops::read::list_versions(block_io, &self.log, path)
    }

    /// Record a named snapshot marker; the returned LSN is a point-in-time handle
    /// a later `O_AT_LSN` open can read the filesystem as of.
    pub fn snapshot<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        name: &str,
        timestamp_ns: u64,
    ) -> Result<Lsn, HelixError> {
        // Payload: [name_len: u16][name]; hash so same-named snapshots correlate.
        let name_b = name.as_bytes();
        let name_hash = crc::fnv1a_64(name_b);
        let mut payload = Vec::with_capacity(2 + name_b.len());
        payload.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        payload.extend_from_slice(name_b);

        let lsn = self
            .log
            .append(LogOp::Snapshot, name_hash, &payload, timestamp_ns)?;
        // Persist so the marker survives a crash and the on-disk LSN is real.
        self.log.flush(block_io)?;
        Ok(lsn)
    }

    /// Flush log; update both superblock slots; flush the device.
    pub fn sync<B: BlockIo>(&mut self, block_io: &mut B) -> Result<(), HelixError> {
        let committed_lsn = self.log.flush(block_io)?;

        self.sb.committed_lsn = committed_lsn;
        self.sb.log_head_segment = self.log.head_segment();
        self.sb.log_head_offset = self.log.head_offset();
        self.sb.log_tail_segment = self.log.tail_segment();

        write_superblock(
            block_io,
            self.partition_lba_start,
            self.device_block_size,
            &mut self.sb,
            0,
        )?;
        write_superblock(
            block_io,
            self.partition_lba_start,
            self.device_block_size,
            &mut self.sb,
            1,
        )?;

        block_io.flush().map_err(|_| HelixError::IoFlushFailed)?;
        Ok(())
    }
}

/// Largest size a `truncate` may *grow* a file to (heap-staged RMW bound).
pub const MAX_TRUNCATE_GROW: u64 = 1 << 20; // 1 MiB
