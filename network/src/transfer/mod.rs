//! Data transfer handling.
//!
//! Provides transfer mechanisms for HTTP:
//! - Chunked transfer encoding decoder
//! - Streaming downloads with progress
//! - Progress tracking utilities

pub mod chunked;
pub mod streaming;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use streaming::{StreamReader, StreamWriter, StreamConfig, StreamState, ProgressTracker};
