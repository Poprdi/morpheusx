//! Namespace B-tree: FNV-1a path_hash -> IndexEntry. In-memory, checkpointed
//! to disk; rebuilt on mount by log replay. Hash collisions resolved by full
//! path compare in leaves.

pub mod btree;
