//! HTTP transfer helpers (chunked decode, streaming buffers) and the
//! download-to-disk orchestrator that drives `state::*`.

pub mod chunked;
pub mod persistence_orchestrator;
pub mod streaming;

pub use chunked::{ChunkedDecoder, DecoderState};
pub use persistence_orchestrator::{
    OrchestratorError, OrchestratorResult, PersistenceConfig, PersistenceOrchestrator,
    PersistencePhase, PersistenceProgress, PersistenceResult,
};
pub use streaming::{ProgressTracker, StreamConfig, StreamReader, StreamState, StreamWriter};
