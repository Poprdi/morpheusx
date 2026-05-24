//! HTTP transfer + post-EBS allocation-free disk I/O (GPT, FAT32, ISO writer, manifest).

pub mod chunked;
pub mod orchestrator;
pub mod streaming;

pub mod disk;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use orchestrator::{
    OrchestratorError, OrchestratorResult, PersistenceConfig, PersistenceOrchestrator,
    PersistencePhase, PersistenceProgress, PersistenceResult,
};
pub use streaming::{ProgressTracker, StreamConfig, StreamReader, StreamState, StreamWriter};

pub use disk::{
    ChunkPartition, ChunkSet, DiskError, DiskResult, Fat32Formatter, Fat32Info, GptOps,
    IsoManifestInfo, IsoWriter, ManifestReader, ManifestWriter, PartitionInfo,
};
