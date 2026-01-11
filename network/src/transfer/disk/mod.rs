//! Post-EBS Disk Operations
//!
//! Allocation-free disk I/O for post-ExitBootServices environment.
//! All operations use fixed-size stack buffers and the VirtIO-blk BlockIo adapter.
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────┐
//! │                  IsoWriter (streaming)                     │
//! │            Coordinates GPT + FAT32 + data writes           │
//! └──────────────────────────┬─────────────────────────────────┘
//!                            │
//!          ┌─────────────────┼─────────────────┐
//!          ▼                 ▼                 ▼
//! ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
//! │  GptOps      │  │  Fat32Fmt    │  │  Manifest    │
//! │  (partition) │  │  (format)    │  │  (tracking)  │
//! └──────────────┘  └──────────────┘  └──────────────┘
//!          │                 │                 │
//!          └─────────────────┼─────────────────┘
//!                            ▼
//! ┌────────────────────────────────────────────────────────────┐
//! │               VirtioBlkBlockIo (driver adapter)            │
//! └────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Design Principles
//!
//! 1. **No heap allocations** - All buffers are stack-allocated or DMA regions
//! 2. **Streaming writes** - Data written as it arrives from HTTP download
//! 3. **Chunk partitions** - ISO split across FAT32 partitions (4GB limit each)
//! 4. **Manifest tracking** - Binary manifest for bootloader to find chunks

mod fat32;
mod gpt;
mod manifest;
mod types;
mod writer;

pub use fat32::{Fat32Formatter, Fat32Info};
pub use gpt::GptOps;
pub use manifest::{IsoManifestInfo, ManifestReader, ManifestWriter};
pub use types::{
    ChunkPartition, ChunkSet, DiskError, DiskResult, PartitionInfo, MAX_CHUNK_PARTITIONS,
    SECTOR_SIZE,
};
pub use writer::IsoWriter;
