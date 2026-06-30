//! Shared host test harness: a writable RAM-backed `BlockIo`.
//!
//! Real disks present 512-byte sectors while HelixFS works in 4 KiB blocks, so
//! the device uses 512-byte sectors deliberately — that exercises the
//! block/sector scaling math (`scale = 8`) on every test, which is exactly
//! where off-by-`scale` bugs hide.

#![allow(dead_code)]

use core::fmt;
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

pub const SECTOR: usize = 512;

/// RAM disk. Writes land immediately; `flush` is a no-op (nothing is buffered).
pub struct MemBio {
    data: Vec<u8>,
}

#[derive(Debug)]
pub struct MemBioError(pub &'static str);

impl fmt::Display for MemBioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemBioError: {}", self.0)
    }
}

impl MemBio {
    /// A zeroed disk of `sectors` × 512 bytes.
    pub fn new(sectors: usize) -> Self {
        Self {
            data: vec![0u8; sectors * SECTOR],
        }
    }

    pub fn sectors(&self) -> u64 {
        (self.data.len() / SECTOR) as u64
    }

    /// Overwrite raw device bytes — lets a test corrupt a specific on-disk field.
    pub fn poke(&mut self, byte_off: usize, bytes: &[u8]) {
        self.data[byte_off..byte_off + bytes.len()].copy_from_slice(bytes);
    }

    /// Read raw device bytes — lets a test inspect an on-disk structure.
    pub fn peek(&self, byte_off: usize, len: usize) -> Vec<u8> {
        self.data[byte_off..byte_off + len].to_vec()
    }
}

impl BlockIo for MemBio {
    type Error = MemBioError;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.sectors())
    }

    fn read_blocks(&mut self, start: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        let end = off + dst.len();
        if end > self.data.len() {
            return Err(MemBioError("read out of range"));
        }
        dst.copy_from_slice(&self.data[off..end]);
        Ok(())
    }

    fn write_blocks(&mut self, start: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        let end = off + src.len();
        if end > self.data.len() {
            return Err(MemBioError("write out of range"));
        }
        self.data[off..end].copy_from_slice(src);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Write-back-cache disk for crash-consistency tests. Writes land in a volatile
/// cache; only `flush()` commits the cache to durable media. `crash()` models a
/// power cut: every write since the last `flush()` evaporates. This makes
/// `flush()` a real barrier, so a test can prove what does (and does not)
/// survive an untimely crash.
pub struct CrashBio {
    durable: Vec<u8>,
    cache: Vec<u8>,
}

impl CrashBio {
    pub fn new(sectors: usize) -> Self {
        let bytes = sectors * SECTOR;
        Self {
            durable: vec![0u8; bytes],
            cache: vec![0u8; bytes],
        }
    }

    pub fn sectors(&self) -> u64 {
        (self.cache.len() / SECTOR) as u64
    }

    /// Drop every write since the last `flush()` (volatile cache lost on power cut).
    pub fn crash(&mut self) {
        self.cache.copy_from_slice(&self.durable);
    }

    /// Overwrite a 512-byte sector in BOTH cache and durable media — models
    /// on-media corruption (bit rot / torn write) the FS must survive on read.
    pub fn corrupt_sector(&mut self, lba: u64, fill: u8) {
        let off = lba as usize * SECTOR;
        for b in &mut self.cache[off..off + SECTOR] {
            *b = fill;
        }
        self.durable[off..off + SECTOR].copy_from_slice(&self.cache[off..off + SECTOR]);
    }
}

impl BlockIo for CrashBio {
    type Error = MemBioError;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.sectors())
    }

    fn read_blocks(&mut self, start: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        let end = off + dst.len();
        if end > self.cache.len() {
            return Err(MemBioError("read out of range"));
        }
        dst.copy_from_slice(&self.cache[off..end]);
        Ok(())
    }

    fn write_blocks(&mut self, start: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        let end = off + src.len();
        if end > self.cache.len() {
            return Err(MemBioError("write out of range"));
        }
        self.cache[off..end].copy_from_slice(src);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.durable.copy_from_slice(&self.cache);
        Ok(())
    }
}
