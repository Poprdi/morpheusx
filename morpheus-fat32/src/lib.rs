//! FAT32 read-only filesystem engine for MorpheusX.
//!
//! Pure engine: generic over `gpt_disk_io::BlockIo`, no knowledge of the
//! kernel VFS, mount table, or `FsBackend`. The storage subsystem wraps this
//! behind its adapter. Read-only by contract — every mutator returns
//! `Fat32Error::ReadOnly`.
//!
//! Invariants:
//! - The backend owns the whole device; all sector math is partition-relative
//!   and offset by `lba_start` before touching the device.
//! - Cluster-chain walks are bounded by `Bpb::cluster_count` so a corrupt FAT
//!   link can never spin forever.

#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod bpb;
pub mod dir;
pub mod error;
pub mod types;

use alloc::vec;
use alloc::vec::Vec;
use bpb::Bpb;
use dir::parse_entries;
use error::Fat32Error;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;
use types::*;

/// Opaque per-fd cookie (start cluster + byte cursor). The kernel stores this
/// in the backend-private region of `FdState`; it must fit `FD_COOKIE_LEN`.
#[derive(Debug, Clone, Copy)]
pub struct Fat32Cookie {
    pub start_cluster: u32,
    pub cursor: u64,
    pub size: u64,
}

pub struct Fat32Fs<B: BlockIo> {
    dev: B,
    bpb: Bpb,
    /// Partition start, in absolute device sectors.
    lba_start: u64,
}

impl<B: BlockIo> Fat32Fs<B> {
    /// Mount: read the boot sector at `lba_start` and parse the BPB. The
    /// device block size must match the BPB's bytes-per-sector.
    pub fn open(mut dev: B, lba_start: u64) -> Result<Self, Fat32Error> {
        let dev_bs = dev.block_size().to_u32();
        let mut sec = vec![0u8; dev_bs as usize];
        Self::read_abs(&mut dev, lba_start, &mut sec)?;

        let bpb = Bpb::parse(&sec)?;
        if bpb.bytes_per_sector != dev_bs {
            return Err(Fat32Error::InvalidBlockSize);
        }
        Ok(Self {
            dev,
            bpb,
            lba_start,
        })
    }

    pub fn capabilities_writable(&self) -> bool {
        false
    }

    fn read_abs(dev: &mut B, lba: u64, dst: &mut [u8]) -> Result<(), Fat32Error> {
        dev.read_blocks(Lba(lba), dst)
            .map_err(|_| Fat32Error::IoRead)
    }

    /// Read `count` partition-relative sectors starting at `rel_sector`.
    fn read_sectors(&mut self, rel_sector: u32, count: u32) -> Result<Vec<u8>, Fat32Error> {
        let bs = self.bpb.bytes_per_sector as usize;
        let total = (count as usize)
            .checked_mul(bs)
            .ok_or(Fat32Error::BadGeometry)?;
        let mut buf = vec![0u8; total];
        let abs = self
            .lba_start
            .checked_add(rel_sector as u64)
            .ok_or(Fat32Error::InvalidOffset)?;
        Self::read_abs(&mut self.dev, abs, &mut buf)?;
        Ok(buf)
    }

    /// Follow one FAT link. Reads the single sector holding the entry rather
    /// than caching the whole FAT — keeps memory flat for huge volumes.
    fn next_cluster(&mut self, cluster: u32) -> Result<u32, Fat32Error> {
        let bs = self.bpb.bytes_per_sector;
        let fat_byte = (cluster as u64)
            .checked_mul(4)
            .ok_or(Fat32Error::ChainCorrupt)?;
        let sector_in_fat = (fat_byte / bs as u64) as u32;
        let off_in_sector = (fat_byte % bs as u64) as usize;

        let rel = self
            .bpb
            .fat_start_sector()
            .checked_add(sector_in_fat)
            .ok_or(Fat32Error::ChainCorrupt)?;
        let sec = self.read_sectors(rel, 1)?;
        let raw = u32::from_le_bytes([
            sec[off_in_sector],
            sec[off_in_sector + 1],
            sec[off_in_sector + 2],
            sec[off_in_sector + 3],
        ]);
        Ok(raw & FAT_ENTRY_MASK)
    }

    fn is_eoc(link: u32) -> bool {
        link >= FAT32_EOC
    }

    /// Collect a cluster chain, bounded by total cluster count so a self-
    /// referential or out-of-range link can never loop.
    fn chain(&mut self, start: u32) -> Result<Vec<u32>, Fat32Error> {
        let mut out = Vec::new();
        if start < FIRST_DATA_CLUSTER {
            return Err(Fat32Error::ChainCorrupt);
        }
        let max = self.bpb.cluster_count().saturating_add(FIRST_DATA_CLUSTER);
        let mut cur = start;
        loop {
            if cur < FIRST_DATA_CLUSTER || cur >= max || cur == FAT32_BAD {
                return Err(Fat32Error::ChainCorrupt);
            }
            out.push(cur);
            if out.len() as u32 > self.bpb.cluster_count() {
                return Err(Fat32Error::ChainCorrupt);
            }
            let next = self.next_cluster(cur)?;
            if Self::is_eoc(next) {
                break;
            }
            cur = next;
        }
        Ok(out)
    }

    fn read_cluster(&mut self, cluster: u32) -> Result<Vec<u8>, Fat32Error> {
        let rel = self.bpb.cluster_to_sector(cluster);
        self.read_sectors(rel, self.bpb.sectors_per_cluster)
    }

    /// Read a whole directory's clusters into one blob.
    fn read_dir_blob(&mut self, start_cluster: u32) -> Result<Vec<u8>, Fat32Error> {
        let chain = self.chain(start_cluster)?;
        let mut blob = Vec::new();
        for c in chain {
            blob.extend_from_slice(&self.read_cluster(c)?);
        }
        Ok(blob)
    }

    fn root_cluster(&self) -> u32 {
        self.bpb.root_cluster
    }

    /// Resolve a `/`-separated absolute path to its directory entry.
    /// Returns the stat for the final component; `/` itself maps to the root.
    fn resolve(&mut self, path: &str) -> Result<FileStat, Fat32Error> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok(FileStat {
                file_type: FileType::Directory,
                size: 0,
                start_cluster: self.root_cluster(),
            });
        }

        let mut cur_cluster = self.root_cluster();
        let mut cur_stat = FileStat {
            file_type: FileType::Directory,
            size: 0,
            start_cluster: cur_cluster,
        };

        let mut iter = path.split('/').filter(|s| !s.is_empty()).peekable();
        while let Some(comp) = iter.next() {
            if comp.len() > 255 {
                return Err(Fat32Error::PathTooLong);
            }
            let blob = self.read_dir_blob(cur_cluster)?;
            let entries = parse_entries(&blob);
            let found = entries
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(comp))
                .ok_or(Fat32Error::NotFound)?;

            let is_last = iter.peek().is_none();
            if !is_last && found.file_type != FileType::Directory {
                return Err(Fat32Error::NotADirectory);
            }
            cur_cluster = found.start_cluster;
            cur_stat = FileStat {
                file_type: found.file_type,
                size: found.size,
                start_cluster: found.start_cluster,
            };
        }
        Ok(cur_stat)
    }

    pub fn stat(&mut self, path: &str) -> Result<FileStat, Fat32Error> {
        self.resolve(path)
    }

    pub fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, Fat32Error> {
        let st = self.resolve(path)?;
        if st.file_type != FileType::Directory {
            return Err(Fat32Error::NotADirectory);
        }
        // Empty dirs report start_cluster 0 in some formatters; treat as empty.
        if st.start_cluster < FIRST_DATA_CLUSTER {
            return Ok(Vec::new());
        }
        let blob = self.read_dir_blob(st.start_cluster)?;
        Ok(parse_entries(&blob))
    }

    /// Open for read: resolve the path, reject directories, return a cookie.
    pub fn open_file(&mut self, path: &str) -> Result<Fat32Cookie, Fat32Error> {
        let st = self.resolve(path)?;
        if st.file_type == FileType::Directory {
            return Err(Fat32Error::IsADirectory);
        }
        Ok(Fat32Cookie {
            start_cluster: st.start_cluster,
            cursor: 0,
            size: st.size,
        })
    }

    /// Read up to `buf.len()` bytes at the cookie's cursor, advancing it.
    /// Returns bytes copied (0 at EOF).
    pub fn read(&mut self, cookie: &mut Fat32Cookie, buf: &mut [u8]) -> Result<usize, Fat32Error> {
        if cookie.cursor >= cookie.size {
            return Ok(0);
        }
        let bpc = self.bpb.bytes_per_cluster() as u64;
        if bpc == 0 {
            return Err(Fat32Error::BadGeometry);
        }
        // Zero-length files have no allocated chain.
        if cookie.size == 0 || cookie.start_cluster < FIRST_DATA_CLUSTER {
            return Ok(0);
        }

        let chain = self.chain(cookie.start_cluster)?;
        let remaining = cookie.size - cookie.cursor;
        let want = (buf.len() as u64).min(remaining);
        let mut copied = 0u64;

        while copied < want {
            let abs_off = cookie.cursor + copied;
            let cluster_idx = (abs_off / bpc) as usize;
            let in_cluster = (abs_off % bpc) as usize;
            let cluster = *chain.get(cluster_idx).ok_or(Fat32Error::ChainCorrupt)?;

            let data = self.read_cluster(cluster)?;
            let avail = (bpc as usize) - in_cluster;
            let need = (want - copied) as usize;
            let n = avail.min(need);
            let dst = &mut buf[copied as usize..copied as usize + n];
            dst.copy_from_slice(&data[in_cluster..in_cluster + n]);
            copied += n as u64;
        }

        cookie.cursor += copied;
        Ok(copied as usize)
    }
}

#[cfg(test)]
mod tests;
