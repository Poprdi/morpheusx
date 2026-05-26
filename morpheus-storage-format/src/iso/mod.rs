//! ISO chunk storage. Splits >4 GB ISOs across multiple FAT32 partitions
//! (one file per partition). Manifest at ESP:`/.iso/<name>.manifest`.
//! Limits: 4 GB - 1 per chunk (FAT32), 16 chunks per ISO.

#![allow(dead_code)]

mod adapter;
mod chunk;
mod error;
mod iso9660_bridge;
mod manifest;
mod reader;
mod storage;
mod writer;

pub use adapter::{ChunkedBlockIo, ChunkedReader, VirtualBlockIo};
pub use chunk::{ChunkInfo, ChunkSet, MAX_CHUNKS};
pub use error::IsoError;
pub use iso9660_bridge::{ChunkedIso, IsoBlockIoAdapter};
pub use manifest::{IsoManifest, MANIFEST_MAGIC, MAX_MANIFEST_SIZE};
pub use reader::{ChunkReader, IsoReadContext};
pub use storage::{IsoEntry, IsoStorageManager, PartitionRequest, MANIFEST_DIR, MAX_ISOS};
pub use writer::{ChunkWriter, WriterState};

pub const FAT32_MAX_FILE_SIZE: u64 = 0xFFFFFFFF;

/// 4 GB minus a page, to leave room for FAT structures.
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024 * 1024 - 4096;

pub const fn chunks_needed(iso_size: u64, chunk_size: u64) -> usize {
    ((iso_size + chunk_size - 1) / chunk_size) as usize
}

/// Includes ~1% per-chunk FAT32 overhead.
pub const fn disk_space_needed(iso_size: u64, chunk_size: u64) -> u64 {
    let num_chunks = chunks_needed(iso_size, chunk_size) as u64;
    iso_size + (num_chunks * (chunk_size / 100))
}
