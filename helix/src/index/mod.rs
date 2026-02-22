//! B-tree namespace index for HelixFS.
//!
//! The namespace index maps `path_hash` (FNV-1a 64-bit) → [`IndexEntry`].
//! It is an in-memory B-tree that is periodically checkpointed to disk.
//!
//! ## Design
//!
//! - Keys are 64-bit path hashes.
//! - Collisions (different paths, same hash) are handled by storing the
//!   full path in each leaf entry and doing a secondary path comparison
//!   on lookup.
//! - The tree is rebuilt on mount by replaying the log from the last
//!   checkpoint.  Therefore, insert/delete/update are purely in-memory
//!   operations during normal use.
//! - On checkpoint, the entire tree is serialized to disk blocks.

pub mod btree;
