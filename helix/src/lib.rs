//! HelixFS — log-structured, versioned FS for MorpheusX.
//!
//! Append-only circular log of `LogRecord`s keyed by monotonic LSN. A B-tree
//! namespace index maps path hashes to the latest `IndexEntry`.
//!
//! Invariants:
//! - Dual superblock: recovery picks highest valid `committed_lsn`.
//! - Log is append-only; records validated by CRC32C; first bad CRC ends scan.
//! - On-disk B-tree only flushed via Checkpoint records; otherwise in RAM.
//! - Three-writes rule: data → flush → pointer → flush.
//! - Every Write/Append carries CRC64 of payload for dedup.

#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod bitmap;
pub mod crc;
pub mod device;
pub mod error;
pub mod format;
pub mod index;
pub mod log;
pub mod ops;
pub mod types;
pub mod vfs;
