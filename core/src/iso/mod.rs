//! ISO Storage Management
//!
//! Manages large ISO files that exceed the FAT32 4GB file size limit by
//! splitting them across multiple "chunk" partitions. Each chunk is stored
//! as a single file on a dedicated FAT32 partition.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        GPT Disk Layout                          │
//! ├────────┬────────┬────────┬────────┬────────┬────────┬──────────┤
//! │  ESP   │ Chunk  │ Chunk  │ Chunk  │ Chunk  │ Chunk  │   Free   │
//! │ (EFI)  │   0    │   1    │   2    │   3    │   4    │  Space   │
//! │ ~512MB │ ~4GB   │ ~4GB   │ ~4GB   │ ~4GB   │ <4GB   │          │
//! └────────┴────────┴────────┴────────┴────────┴────────┴──────────┘
//! ```
//!
//! # Manifest Format
//!
//! Each ISO set has a manifest file stored on the ESP at `/morpheus/isos/<name>.manifest`:
//!
//! ```text
//! MORPHEUS_ISO_MANIFEST_V1
//! name: ubuntu-24.04-desktop-amd64.iso
//! size: 6234567890
//! chunks: 3
//! sha256: <hash>
//! chunk_0: <partition_uuid> 4294967296
//! chunk_1: <partition_uuid> 4294967296
//! chunk_2: <partition_uuid> 1939600298
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // Writing an ISO (during download)
//! let mut writer = ChunkWriter::new(block_io, &manifest)?;
//! writer.write_chunk_data(data_slice)?;
//! writer.finalize()?;
//!
//! // Reading an ISO (during boot)
//! let reader = ChunkReader::new(block_io, &manifest)?;
//! let data = reader.read_range(offset, length)?;
//! ```
//!
//! # Constraints
//!
//! - Maximum chunk size: 4GB - 1 byte (FAT32 limit)
//! - Maximum chunks per ISO: 16 (fixed array)
//! - Chunk partition type: Linux filesystem GUID (for now)
//! - Each chunk partition is formatted as FAT32 with single file

#![allow(dead_code)] // Module under construction

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
pub use iso9660_bridge::{IsoBlockIoAdapter, ChunkedIso};
pub use manifest::{IsoManifest, MANIFEST_MAGIC, MAX_MANIFEST_SIZE};
pub use reader::{ChunkReader, IsoReadContext};
pub use storage::{IsoStorageManager, IsoEntry, PartitionRequest, MAX_ISOS, MANIFEST_DIR};
pub use writer::{ChunkWriter, WriterState};

/// Maximum file size that FAT32 supports (4GB - 1 byte)
pub const FAT32_MAX_FILE_SIZE: u64 = 0xFFFFFFFF; // 4,294,967,295 bytes

/// Default chunk size (slightly under 4GB to allow for FAT32 overhead)
pub const DEFAULT_CHUNK_SIZE: u64 = 4 * 1024 * 1024 * 1024 - 4096; // 4GB - 4KB

/// Calculate number of chunks needed for a given ISO size
pub const fn chunks_needed(iso_size: u64, chunk_size: u64) -> usize {
    ((iso_size + chunk_size - 1) / chunk_size) as usize
}

/// Calculate total disk space needed for an ISO (with FAT32 overhead)
pub const fn disk_space_needed(iso_size: u64, chunk_size: u64) -> u64 {
    let num_chunks = chunks_needed(iso_size, chunk_size) as u64;
    // Add ~1% overhead per chunk for FAT32 structures
    iso_size + (num_chunks * (chunk_size / 100))
}
