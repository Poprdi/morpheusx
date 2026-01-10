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

mod types;
mod gpt;
mod fat32;
mod writer;
mod manifest;

pub use types::{
    DiskError, DiskResult, PartitionInfo, ChunkPartition,
    SECTOR_SIZE, MAX_CHUNK_PARTITIONS,
};
pub use gpt::GptOps;
pub use fat32::Fat32Formatter;
pub use writer::IsoWriter;
pub use manifest::ManifestWriter;
