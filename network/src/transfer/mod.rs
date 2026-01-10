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
pub mod streaming;
pub mod orchestrator;

// Post-EBS disk operations (allocation-free, modular)
pub mod disk;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use streaming::{StreamReader, StreamWriter, StreamConfig, StreamState, ProgressTracker};
pub use orchestrator::{
    PersistenceOrchestrator, PersistenceConfig, PersistenceProgress, 
    PersistencePhase, PersistenceResult, OrchestratorResult, OrchestratorError,
};

// Re-export disk module types for convenience
pub use disk::{
    DiskError, DiskResult, PartitionInfo, ChunkPartition, ChunkSet,
    GptOps, Fat32Formatter, Fat32Info, IsoWriter, ManifestWriter, ManifestReader, IsoManifestInfo,
};
