//! Data transfer handling.
//!
//! Provides transfer mechanisms for HTTP:
//! - Chunked transfer encoding decoder
//! - Streaming downloads with progress
//! - Progress tracking utilities
//! - End-to-end orchestration
//!
//! # Post-EBS Disk Operations (new modular approach)
//!
//! The `disk` submodule provides allocation-free disk I/O for post-ExitBootServices:
//! - GPT partition creation/scanning
//! - FAT32 formatting
//! - Streaming ISO writer with chunking
//! - Binary manifest for bootloader integration

pub mod chunked;
pub mod orchestrator;
pub mod streaming;

// Post-EBS disk operations (allocation-free, modular)
pub mod disk;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use orchestrator::{
    OrchestratorError, OrchestratorResult, PersistenceConfig, PersistenceOrchestrator,
    PersistencePhase, PersistenceProgress, PersistenceResult,
};
pub use streaming::{ProgressTracker, StreamConfig, StreamReader, StreamState, StreamWriter};

// Re-export disk module types for convenience
pub use disk::{
    ChunkPartition, ChunkSet, DiskError, DiskResult, Fat32Formatter, Fat32Info, GptOps,
    IsoManifestInfo, IsoWriter, ManifestReader, ManifestWriter, PartitionInfo,
};
