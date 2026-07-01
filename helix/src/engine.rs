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
    /// LSNs of live snapshots, ascending; an overwritten version is retained
    /// while a snapshot in `[old_lsn, new_lsn)` still references it.
    pub snapshot_lsns: Vec<Lsn>,
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
            snapshot_lsns: Vec::new(),
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
        // A checkpoint captures the namespace as of checkpoint_lsn; load it, then
        // replay only the records logged after it.
        if fs.sb.index_root_block != BLOCK_NULL {
            crate::checkpoint::load_index_region(
                block_io,
                fs.partition_lba_start,
                fs.sb.data_start_block,
                fs.device_block_size,
                fs.sb.index_root_block,
                fs.sb.index_depth as u64,
                fs.sb.index_entry_count as u64,
                &mut fs.index,
            )?;
        }
        let mut snapshots = Vec::new();
        replay_log(
            block_io,
            &fs.log,
            &mut fs.index,
            &mut snapshots,
            fs.sb.checkpoint_lsn,
        )?;
        fs.snapshot_lsns = snapshots;
        fs.rebuild_bitmap_from_index(block_io);
        Ok(fs)
    }

    /// After replay the bitmap is zero; mark every extent-backed live file's
    /// blocks (and its extent-node block) used or new allocations would overlap
    /// existing data.
    fn rebuild_bitmap_from_index<B: BlockIo>(&mut self, block_io: &mut B) {
        // Snapshot the targets first so the index borrow doesn't overlap the
        // bitmap/device access below.
        let targets: Vec<(BlockAddr, u64, bool)> = self
            .index
            .all_entries()
            .iter()
            .filter(|e| {
                e.flags & (entry_flags::IS_DELETED | entry_flags::IS_DIR | entry_flags::IS_INLINE)
                    == 0
                    && e.extent_root != BLOCK_NULL
            })
            .map(|e| {
                (
                    e.extent_root,
                    e.size,
                    e.flags & entry_flags::IS_EXTENT_NODE != 0,
                )
            })
            .collect();

        for (extent_root, size, is_node) in targets {
            if is_node {
                self.bitmap.mark_block_used(extent_root);
                if let Ok(extents) = crate::extent::read_extent_node(
                    block_io,
                    self.partition_lba_start,
                    self.sb.data_start_block,
                    self.device_block_size,
                    extent_root,
                ) {
                    for (_, physical, count) in extents {
                        self.bitmap.mark_range_used(physical, count as u64);
                    }
                }
            } else {
                let blocks_needed = size.div_ceil(BLOCK_SIZE as u64);
                if blocks_needed > 0 {
                    self.bitmap.mark_range_used(extent_root, blocks_needed);
                }
            }
        }

        // The on-disk index checkpoint region is also live storage.
        if self.sb.index_root_block != BLOCK_NULL {
            self.bitmap
                .mark_range_used(self.sb.index_root_block, self.sb.index_depth as u64);
        }
    }

    /// Resolve `path`: create if `O_CREATE`+absent, truncate if `O_TRUNC`. Returns the index key.
    /// Permission policy is the caller's; the engine enforces only existence and create/trunc.
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
        // Route through write() so O_TRUNC over an existing file reclaims it.
        self.write(block_io, path, &[], timestamp_ns)
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
        // Capture the prior version's blocks before write_file replaces the entry.
        let old = self.index.lookup(path).and_then(|e| {
            let extent_backed = e.flags
                & (entry_flags::IS_INLINE | entry_flags::IS_DIR | entry_flags::IS_DELETED)
                == 0
                && e.extent_root != BLOCK_NULL;
            extent_backed.then_some((
                e.extent_root,
                e.size,
                e.lsn,
                e.flags & entry_flags::IS_EXTENT_NODE != 0,
            ))
        });

        let new_lsn = self.write_file_checkpointing(block_io, path, data, timestamp_ns)?;

        // Reclaim the superseded version unless a snapshot still references it.
        if let Some((extent_root, size, old_lsn, is_node)) = old {
            if !self.snapshot_pins(old_lsn, new_lsn) {
                crate::extent::free_file_blocks(
                    block_io,
                    &mut self.bitmap,
                    self.partition_lba_start,
                    self.sb.data_start_block,
                    self.device_block_size,
                    extent_root,
                    size,
                    is_node,
                );
            }
        }
        Ok(())
    }

    /// A prior version is pinned if a snapshot was taken while it was current —
    /// a snapshot LSN in `[old_lsn, new_lsn)`.
    fn snapshot_pins(&self, old_lsn: Lsn, new_lsn: Lsn) -> bool {
        self.snapshot_lsns
            .iter()
            .any(|&s| s >= old_lsn && s < new_lsn)
    }

    fn write_file_inner<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        data: &[u8],
        timestamp_ns: u64,
    ) -> Result<Lsn, HelixError> {
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
        )
    }

    /// `write_file`, but a full log triggers a checkpoint (which recycles the
    /// ring) and one retry instead of bricking.
    fn write_file_checkpointing<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        data: &[u8],
        timestamp_ns: u64,
    ) -> Result<Lsn, HelixError> {
        match self.write_file_inner(block_io, path, data, timestamp_ns) {
            Err(HelixError::LogFull) => {
                self.checkpoint(block_io)?;
                self.write_file_inner(block_io, path, data, timestamp_ns)
            },
            other => other,
        }
    }

    /// Run a mutation; on a full log, checkpoint to recycle the ring and retry
    /// once. Mutations roll back cleanly on `LogFull` (append is their first
    /// fallible step), so the retry is safe.
    fn with_checkpoint_retry<B, F, T>(
        &mut self,
        block_io: &mut B,
        mut op: F,
    ) -> Result<T, HelixError>
    where
        B: BlockIo,
        F: FnMut(&mut Self, &mut B) -> Result<T, HelixError>,
    {
        match op(self, block_io) {
            Err(HelixError::LogFull) => {
                self.checkpoint(block_io)?;
                op(self, block_io)
            },
            other => other,
        }
    }

    /// Persist the live namespace to the on-disk index region and recycle the
    /// log ring. Crash-safe: the new region is durable before the superblock
    /// that points at it, and the old region is freed only afterward.
    pub fn checkpoint<B: BlockIo>(&mut self, block_io: &mut B) -> Result<(), HelixError> {
        let live: Vec<IndexEntry> = self
            .index
            .all_entries()
            .iter()
            .filter(|e| e.flags & entry_flags::IS_DELETED == 0)
            .copied()
            .collect();
        let blocks_needed = crate::checkpoint::region_blocks(live.len());

        let region_start = self.bitmap.alloc_contiguous(blocks_needed)?;
        if let Err(e) = crate::checkpoint::write_index_region(
            block_io,
            self.partition_lba_start,
            self.sb.data_start_block,
            self.device_block_size,
            region_start,
            &live,
        ) {
            let _ = self.bitmap.free_range(region_start, blocks_needed);
            return Err(e);
        }

        let old_root = self.sb.index_root_block;
        let old_blocks = self.sb.index_depth as u64;

        let checkpoint_lsn = self.log.next_lsn().saturating_sub(1);
        self.log.reset_ring();

        self.sb.index_root_block = region_start;
        self.sb.index_depth = blocks_needed as u32;
        self.sb.index_entry_count = live.len() as u32;
        self.sb.checkpoint_lsn = checkpoint_lsn;
        self.sb.committed_lsn = self.log.flush(block_io)?;
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

        if old_root != BLOCK_NULL {
            let _ = self.bitmap.free_range(old_root, old_blocks);
        }
        Ok(())
    }

    pub fn stat(&self, path: &str) -> Result<FileStat, HelixError> {
        ops::read::stat_file(&self.index, path)
    }

    pub fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, HelixError> {
        ops::dir::readdir(&self.index, path)
    }

    pub fn mkdir<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        self.with_checkpoint_retry(block_io, |s, dev| {
            ops::dir::mkdir(dev, &mut s.log, &mut s.index, path, timestamp_ns).map(|_| ())
        })
    }

    /// File or empty directory.
    pub fn unlink<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        self.with_checkpoint_retry(block_io, |s, dev| {
            ops::dir::unlink(
                dev,
                &mut s.log,
                &mut s.index,
                &mut s.bitmap,
                s.partition_lba_start,
                s.sb.data_start_block,
                s.device_block_size,
                path,
                timestamp_ns,
            )
            .map(|_| ())
        })
    }

    pub fn rename<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        old_path: &str,
        new_path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        self.with_checkpoint_retry(block_io, |s, dev| {
            ops::write::rename(
                dev,
                &mut s.log,
                &mut s.index,
                &mut s.bitmap,
                s.partition_lba_start,
                s.sb.data_start_block,
                s.device_block_size,
                old_path,
                new_path,
                timestamp_ns,
            )
            .map(|_| ())
        })
    }

    /// Resize `path` via log-structured RMW. Grows capped at `MAX_TRUNCATE_GROW`
    /// to prevent OOM on the small kernel heap.
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

    /// All logged versions of `path`, oldest-to-newest.
    pub fn versions<B: BlockIo>(
        &self,
        block_io: &mut B,
        path: &str,
    ) -> Result<Vec<(Lsn, u64, LogOp)>, HelixError> {
        ops::read::list_versions(block_io, &self.log, path)
    }

    /// Append a snapshot marker; returned LSN is the point-in-time handle for `O_AT_LSN`.
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
            .append(block_io, LogOp::Snapshot, name_hash, &payload, timestamp_ns)?;
        self.snapshot_lsns.push(lsn);
        // Persist the log AND the superblock; flushing the log alone leaves the
        // replay boundary behind the marker, so a crash would drop it.
        self.sync(block_io)?;
        Ok(lsn)
    }

    /// Flush log, write both superblock slots, flush device.
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
