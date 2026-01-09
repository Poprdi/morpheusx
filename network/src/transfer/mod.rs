//! Data transfer handling.
//!
//! Provides transfer mechanisms for HTTP:
//! - Chunked transfer encoding decoder
//! - Streaming downloads with progress
//! - Progress tracking utilities
//! - End-to-end orchestration

pub mod chunked;
pub mod streaming;
pub mod orchestrator;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use streaming::{StreamReader, StreamWriter, StreamConfig, StreamState, ProgressTracker};
pub use orchestrator::{
    PersistenceOrchestrator, PersistenceConfig, PersistenceProgress, 
    PersistencePhase, PersistenceResult, OrchestratorResult, OrchestratorError,
};
