//! Post-EBS disk I/O: alloc-free, stack-buffered. IsoWriter coordinates
//! GptOps + Fat32Formatter + ManifestWriter on top of the VirtIO-blk BlockIo
//! adapter. ISOs split across multiple FAT32 partitions to dodge the 4 GB cap.

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
