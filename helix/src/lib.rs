//! HelixFS — Log-structured, time-travel filesystem for MorpheusX.
//!
//! # Architecture
//!
//! Helix is an append-only log-structured filesystem where every write is
//! a versioned record.  The primary data structure is a circular log of
//! [`LogRecord`] entries, each identified by a monotonically increasing
//! Log Sequence Number ([`Lsn`]).  A secondary B-tree *namespace index*
//! maps path hashes to the latest [`IndexEntry`] for O(1) lookups and
//! O(children) directory listings.
//!
//! ## On-disk invariants
//!
//! 1. **Dual superblock**: two copies written alternately.  Recovery
//!    picks the one with the highest `committed_lsn` and a valid CRC.
//! 2. **Log append-only**: records are never modified.  A record is valid
//!    iff its CRC32C passes.  Recovery scans forward from the superblock's
//!    `committed_lsn` and stops at the first invalid CRC.
//! 3. **Checkpoint-guarded index**: the on-disk B-tree is only flushed
//!    as part of a Checkpoint log record.  Between checkpoints the index
//!    is volatile (exists in RAM only).  Recovery replays the log from
//!    the last checkpoint to rebuild the in-memory index.
//! 4. **Three-writes rule**: data → flush → pointer → flush. A valid
//!    pointer never references invalid data.
//! 5. **Content CRC64 dedup**: every `Write`/`Append` record carries a
//!    CRC64 of its payload.  The dedup index can elide duplicate writes.
//!
//! ## Crash recovery
//!
//! ```text
//! mount():
//!   1. Read both superblocks; pick highest valid committed_lsn
//!   2. Load checkpoint B-tree root from superblock
//!   3. Scan log forward from checkpoint LSN
//!      - For each record with valid CRC: apply to in-memory index
//!      - First invalid CRC: stop (partial write, discard)
//!   4. Done — filesystem is consistent
//! ```

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
