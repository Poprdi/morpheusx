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
use crate::retain;
use crate::types::*;
use crate::{crc, format};
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;

/// Extra entries a spare checkpoint region is sized for.
const SPARE_SLACK_ENTRIES: usize = 24;

/// One mounted HelixFS volume. Owns sb/log/index/bitmap and its partition LBA
/// base; all I/O goes through a borrowed `&mut B` so the device stays caller-owned.
pub struct HelixFs {
    pub sb: HelixSuperblock,
    pub log: LogEngine,
    pub index: NamespaceIndex,
    pub bitmap: BlockBitmap,
    pub partition_lba_start: u64,
    pub device_block_size: u32,
    /// LSNs of live snapshots, ascending.
    pub snapshot_lsns: Vec<Lsn>,
    /// Replaced versions a snapshot still references (see `retain`).
    pub retained: Vec<IndexEntry>,
    /// Pre-allocated region for the next checkpoint, so a full disk can still
    /// checkpoint. `(start, blocks)`.
    spare: Option<(u64, u64)>,
}

fn check_device_block_size(dbs: u32) -> Result<(), HelixError> {
    if dbs == 0 || dbs > BLOCK_SIZE || !dbs.is_power_of_two() {
        return Err(HelixError::InvalidBlockSize);
    }
    Ok(())
}

fn snap_region_blocks(count: usize) -> u64 {
    if count == 0 {
        0
    } else {
        crate::checkpoint::region_blocks(count)
    }
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
            retained: Vec::new(),
            spare: None,
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
        check_device_block_size(block_size)?;
        format::format_helix(block_io, lba_start, lba_count, block_size, label, uuid)?;
        Self::mount(block_io, lba_start, block_size)
    }

    /// Recover the superblock, replay the log, rebuild the allocation bitmap.
    /// Any inconsistency in committed state fails the mount.
    pub fn mount<B: BlockIo>(
        block_io: &mut B,
        lba_start: u64,
        block_size: u32,
    ) -> Result<Self, HelixError> {
        check_device_block_size(block_size)?;
        let sb = recover_superblock(block_io, lba_start, block_size)?;
        if sb.version != HELIX_VERSION {
            return Err(HelixError::IncompatibleVersion);
        }
        sb.validate_geometry()?;
        let mut fs = Self::from_superblock(sb, lba_start, block_size);
        // Reload head so flush() doesn't clobber existing records.
        fs.log.reload_head_segment(block_io)?;

        if fs.sb.index_root_block != BLOCK_NULL {
            let index = &mut fs.index;
            crate::checkpoint::load_index_region(
                block_io,
                lba_start,
                fs.sb.data_start_block,
                block_size,
                fs.sb.index_root_block,
                fs.sb.index_entry_count as u64,
                |e| index.upsert(e),
            )?;
            let (snaps, retained) = (&mut fs.snapshot_lsns, &mut fs.retained);
            crate::checkpoint::load_index_region(
                block_io,
                lba_start,
                fs.sb.data_start_block,
                block_size,
                fs.sb.snapshot_table_block,
                fs.sb.snapshot_count as u64,
                |e| {
                    if e.flags & entry_flags::IS_SNAPSHOT_MARK != 0 {
                        snaps.push(e.lsn);
                    } else {
                        retained.push(e);
                    }
                },
            )?;
        }
        replay_log(
            block_io,
            &fs.log,
            &mut fs.index,
            &mut fs.snapshot_lsns,
            &mut fs.retained,
            fs.sb.checkpoint_lsn,
            fs.sb.committed_lsn,
        )?;
        fs.snapshot_lsns.sort_unstable();
        fs.snapshot_lsns.dedup();
        fs.rebuild_bitmap_from_index(block_io)?;
        fs.refill_spare();
        Ok(fs)
    }

    fn checkpoint_region(&self) -> Option<(u64, u64)> {
        (self.sb.index_root_block != BLOCK_NULL).then(|| {
            (
                self.sb.index_root_block,
                self.sb.index_depth as u64 + snap_region_blocks(self.sb.snapshot_count as usize),
            )
        })
    }

    /// The bitmap is not stored; mark every block a live or retained file
    /// owns, plus the checkpoint region.
    fn rebuild_bitmap_from_index<B: BlockIo>(
        &mut self,
        block_io: &mut B,
    ) -> Result<(), HelixError> {
        let targets: Vec<(BlockAddr, u64, bool)> = self
            .index
            .all_entries()
            .iter()
            .chain(self.retained.iter())
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

        let mark = |bitmap: &mut BlockBitmap, start: u64, count: u64| {
            if !bitmap.in_range(start, count) {
                return Err(HelixError::BitmapCorrupt);
            }
            bitmap.mark_range_used(start, count);
            Ok(())
        };

        for (extent_root, size, is_node) in targets {
            if is_node {
                mark(&mut self.bitmap, extent_root, 1)?;
                let extents = crate::extent::read_extent_node(
                    block_io,
                    self.partition_lba_start,
                    self.sb.data_start_block,
                    self.device_block_size,
                    extent_root,
                )?;
                for (_, physical, count) in extents {
                    mark(&mut self.bitmap, physical, count as u64)?;
                }
            } else {
                mark(
                    &mut self.bitmap,
                    extent_root,
                    size.div_ceil(BLOCK_SIZE as u64),
                )?;
            }
        }

        if let Some((start, blocks)) = self.checkpoint_region() {
            mark(&mut self.bitmap, start, blocks)?;
        }
        Ok(())
    }

    fn spare_target(&self) -> u64 {
        crate::checkpoint::region_blocks(self.index.live_count() + SPARE_SLACK_ENTRIES)
            + snap_region_blocks(self.snapshot_lsns.len() + self.retained.len())
    }

    /// Best effort; a missing spare only matters once the disk is full.
    fn refill_spare(&mut self) {
        let want = self.spare_target();
        // not worth it on a volume this small
        if self.spare.is_some_and(|(_, n)| n >= want) || want > self.sb.data_block_count / 32 {
            return;
        }
        if let Ok(start) = self.bitmap.alloc_contiguous(want) {
            if let Some((s, n)) = self.spare.replace((start, want)) {
                let _ = self.bitmap.free_range(s, n);
            }
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
                self.write(block_io, path, &[], timestamp_ns)?;
            } else {
                return Err(HelixError::NotFound);
            }
        } else if flags & open_flags::O_TRUNC != 0 {
            self.write(block_io, path, &[], timestamp_ns)?;
        }

        let idx_entry = self.index.lookup(path).ok_or(HelixError::NotFound)?;
        Ok(idx_entry.key)
    }

    /// Full file contents. The caller applies its own fd offset/slicing.
    pub fn read<B: BlockIo>(&self, block_io: &mut B, path: &str) -> Result<Vec<u8>, HelixError> {
        ops::read::read_file(
            block_io,
            &self.index,
            self.partition_lba_start,
            self.sb.data_start_block,
            self.device_block_size,
            path,
        )
    }

    /// Contents of `path` as of `lsn`. Only versions that are still live or
    /// pinned by a snapshot can be returned; anything else is `NotFound`.
    pub fn read_at_lsn<B: BlockIo>(
        &self,
        block_io: &mut B,
        path: &str,
        lsn: Lsn,
    ) -> Result<Vec<u8>, HelixError> {
        let entry = match self.index.lookup(path) {
            Some(e) if e.lsn <= lsn => e,
            _ => retain::version_at(&self.retained, path, lsn).ok_or(HelixError::NotFound)?,
        };
        ops::read::read_entry(
            block_io,
            self.partition_lba_start,
            self.sb.data_start_block,
            self.device_block_size,
            entry,
        )
    }

    /// Overwrite `path` with `data` (log-structured: a new version is appended).
    pub fn write<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        path: &str,
        data: &[u8],
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        let old = self.index.lookup(path).copied();
        let new_lsn = self.with_retry(block_io, |s, dev| {
            ops::write::write_file(
                dev,
                &mut s.log,
                &mut s.index,
                &mut s.bitmap,
                s.partition_lba_start,
                s.device_block_size,
                s.sb.data_start_block,
                path,
                data,
                timestamp_ns,
            )
        })?;
        if let Some(old) = old {
            self.release_version(block_io, &old, new_lsn);
        }
        Ok(())
    }

    /// Dispose of a version replaced at `new_lsn`: keep it if a snapshot pins
    /// it, otherwise free its blocks. Blocks the committed superblock can
    /// still reach are only released at the next commit.
    fn release_version<B: BlockIo>(&mut self, block_io: &mut B, old: &IndexEntry, new_lsn: Lsn) {
        if retain::retire(&self.snapshot_lsns, &mut self.retained, old, new_lsn) {
            return;
        }
        let owns_blocks = old.flags
            & (entry_flags::IS_INLINE | entry_flags::IS_DIR | entry_flags::IS_DELETED)
            == 0
            && old.extent_root != BLOCK_NULL;
        if !owns_blocks {
            return;
        }
        crate::extent::free_file_blocks(
            block_io,
            &mut self.bitmap,
            self.partition_lba_start,
            self.sb.data_start_block,
            self.device_block_size,
            old.extent_root,
            old.size,
            old.flags & entry_flags::IS_EXTENT_NODE != 0,
            old.lsn <= self.sb.committed_lsn,
        );
    }

    /// Run a mutation, recovering from the two resource errors that have a
    /// remedy: a full log (checkpoint) and blocks held by deferred frees (sync).
    fn with_retry<B, F, T>(&mut self, block_io: &mut B, mut op: F) -> Result<T, HelixError>
    where
        B: BlockIo,
        F: FnMut(&mut Self, &mut B) -> Result<T, HelixError>,
    {
        let mut checkpointed = false;
        let mut synced = false;
        loop {
            match op(self, block_io) {
                Err(HelixError::LogFull) if !checkpointed => {
                    checkpointed = true;
                    self.checkpoint(block_io)?;
                },
                Err(HelixError::NoSpace) if !synced && self.bitmap.has_pending() => {
                    synced = true;
                    self.sync(block_io)?;
                },
                other => return other,
            }
        }
    }

    fn refresh_counters(&self, sb: &mut HelixSuperblock) {
        let (mut files, mut dirs) = (0u64, 1u64);
        for e in self.index.all_entries() {
            if e.flags & entry_flags::IS_DELETED != 0 {
                continue;
            }
            if e.flags & entry_flags::IS_DIR != 0 {
                dirs += 1;
            } else {
                files += 1;
            }
        }
        sb.file_count = files;
        sb.dir_count = dirs;
        sb.blocks_used = sb.data_start_block + self.bitmap.allocated_count();
    }

    fn alloc_checkpoint_region<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        need: u64,
    ) -> Result<u64, HelixError> {
        if let Some((start, blocks)) = self.spare.take() {
            if blocks >= need {
                let _ = self.bitmap.free_range(start + need, blocks - need);
                return Ok(start);
            }
            let _ = self.bitmap.free_range(start, blocks);
        }
        match self.bitmap.alloc_contiguous(need) {
            Err(HelixError::NoSpace) if self.bitmap.has_pending() => {
                self.sync(block_io)?;
                self.bitmap.alloc_contiguous(need)
            },
            other => other,
        }
    }

    /// Persist the namespace and snapshot table, then restart the log. The
    /// superblock write is the commit point: nothing the old superblock
    /// references is modified before it.
    pub fn checkpoint<B: BlockIo>(&mut self, block_io: &mut B) -> Result<(), HelixError> {
        let live: Vec<IndexEntry> = self
            .index
            .all_entries()
            .iter()
            .filter(|e| e.flags & entry_flags::IS_DELETED == 0)
            .copied()
            .collect();
        let snap: Vec<IndexEntry> = self
            .snapshot_lsns
            .iter()
            .map(|&l| retain::snapshot_mark(l))
            .chain(self.retained.iter().copied())
            .collect();
        let index_blocks = crate::checkpoint::region_blocks(live.len());
        let need = index_blocks + snap_region_blocks(snap.len());

        let region_start = self.alloc_checkpoint_region(block_io, need)?;
        let (pls, dsb, dbs) = (
            self.partition_lba_start,
            self.sb.data_start_block,
            self.device_block_size,
        );
        let written =
            crate::checkpoint::write_index_region(block_io, pls, dsb, dbs, region_start, &live)
                .and_then(|_| {
                    if snap.is_empty() {
                        return Ok(());
                    }
                    crate::checkpoint::write_index_region(
                        block_io,
                        pls,
                        dsb,
                        dbs,
                        region_start + index_blocks,
                        &snap,
                    )
                })
                .and_then(|_| block_io.flush().map_err(|_| HelixError::IoFlushFailed));
        if let Err(e) = written {
            let _ = self.bitmap.free_range(region_start, need);
            return Err(e);
        }

        let old_region = self.checkpoint_region();
        let lsn = self.log.next_lsn() - 1;

        let mut sb = self.sb;
        sb.index_root_block = region_start;
        sb.index_depth = index_blocks as u32;
        sb.index_entry_count = live.len() as u32;
        sb.snapshot_count = snap.len() as u32;
        sb.snapshot_table_block = if snap.is_empty() {
            BLOCK_NULL
        } else {
            region_start + index_blocks
        };
        sb.checkpoint_lsn = lsn;
        sb.committed_lsn = lsn;
        sb.log_head_segment = self.log.head_segment();
        sb.log_tail_segment = self.log.head_segment();
        sb.log_head_offset = core::mem::size_of::<LogSegmentHeader>() as u32;
        self.refresh_counters(&mut sb);

        // an unknown outcome here may have landed on disk, so the new region
        // is leaked rather than freed
        write_superblock(block_io, pls, dbs, &mut sb, 0)?;

        self.sb = sb;
        self.log.restart_head(0);
        self.bitmap.commit_pending();
        if let Some((start, blocks)) = old_region {
            if self.spare.is_none() && blocks >= self.spare_target() {
                self.spare = Some((start, blocks));
            } else {
                let _ = self.bitmap.free_range(start, blocks);
            }
        }
        self.refill_spare();

        write_superblock(block_io, pls, dbs, &mut self.sb, 1)
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
        self.with_retry(block_io, |s, dev| {
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
        let (lsn, removed) = self.with_retry(block_io, |s, dev| {
            ops::dir::unlink(dev, &mut s.log, &mut s.index, path, timestamp_ns)
        })?;
        self.release_version(block_io, &removed, lsn);
        Ok(())
    }

    pub fn rename<B: BlockIo>(
        &mut self,
        block_io: &mut B,
        old_path: &str,
        new_path: &str,
        timestamp_ns: u64,
    ) -> Result<(), HelixError> {
        // a directory rename is many records; make sure they all fit before
        // the first one so a checkpoint never lands in the middle
        let need =
            ops::write::rename_log_bytes(&self.index, old_path) + 8 * crate::log::MAX_RECORD_BYTES;
        if need > self.log.free_bytes() {
            self.checkpoint(block_io)?;
            if need > self.log.free_bytes() {
                return Err(HelixError::LogFull);
            }
        }
        let (lsn, replaced) = ops::write::rename(
            block_io,
            &mut self.log,
            &mut self.index,
            old_path,
            new_path,
            timestamp_ns,
        )?;
        if let Some(old) = replaced {
            self.release_version(block_io, &old, lsn);
        }
        Ok(())
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
        let cur_size = {
            let e = self.index.lookup(path).ok_or(HelixError::NotFound)?;
            if e.flags & entry_flags::IS_DIR != 0 {
                return Err(HelixError::IsADirectory);
            }
            e.size
        };

        if new_size == cur_size {
            return Ok(());
        }
        if new_size > cur_size && new_size > MAX_TRUNCATE_GROW {
            return Err(HelixError::FileTooLarge);
        }

        let mut data = self.read(block_io, path)?;
        if new_size as usize > data.len() {
            data.try_reserve_exact(new_size as usize - data.len())
                .map_err(|_| HelixError::FileTooLarge)?;
        }
        data.resize(new_size as usize, 0u8);

        self.write(block_io, path, &data, timestamp_ns)
    }

    /// All logged versions of `path`, oldest-to-newest.
    pub fn versions<B: BlockIo>(
        &self,
        block_io: &mut B,
        path: &str,
    ) -> Result<Vec<(Lsn, u64, LogOp)>, HelixError> {
        ops::read::list_versions(block_io, &self.log, path)
    }

    /// Append a snapshot marker; returned LSN is the point-in-time handle for
    /// `read_at_lsn`.
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

        let lsn = self.with_retry(block_io, |s, dev| {
            s.log
                .append(dev, LogOp::Snapshot, name_hash, &payload, timestamp_ns)
        })?;
        self.snapshot_lsns.push(lsn);
        // the superblock has to move past the marker or a crash drops it
        self.sync(block_io)?;
        Ok(lsn)
    }

    /// Flush log, write both superblock slots, flush device. Slot A landing
    /// is the commit point.
    pub fn sync<B: BlockIo>(&mut self, block_io: &mut B) -> Result<(), HelixError> {
        let committed_lsn = self.log.flush(block_io)?;

        let mut sb = self.sb;
        sb.committed_lsn = committed_lsn;
        sb.log_head_segment = self.log.head_segment();
        sb.log_head_offset = self.log.head_offset();
        sb.log_tail_segment = self.log.tail_segment();
        self.refresh_counters(&mut sb);

        let (pls, dbs) = (self.partition_lba_start, self.device_block_size);
        write_superblock(block_io, pls, dbs, &mut sb, 0)?;
        self.sb = sb;
        self.bitmap.commit_pending();
        write_superblock(block_io, pls, dbs, &mut self.sb, 1)
    }
}

/// Largest size a `truncate` may *grow* a file to (heap-staged RMW bound).
pub const MAX_TRUNCATE_GROW: u64 = 1 << 20; // 1 MiB
