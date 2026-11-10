//! Versions kept alive by snapshots. A version is retained when it is replaced
//! while a snapshot taken during its lifetime exists; it carries the LSN that
//! superseded it in `_reserved[..8]`.

use crate::index::btree::path_bytes;
use crate::types::*;
use alloc::vec::Vec;

pub fn superseded_lsn(e: &IndexEntry) -> Lsn {
    let mut b = [0u8; 8];
    b.copy_from_slice(&e._reserved[..8]);
    u64::from_le_bytes(b)
}

/// True if a snapshot was taken while the version `[old_lsn, new_lsn)` was current.
pub fn pins(snapshots: &[Lsn], old_lsn: Lsn, new_lsn: Lsn) -> bool {
    snapshots.iter().any(|&s| s >= old_lsn && s < new_lsn)
}

/// Keep `old` if a snapshot pins it. Returns true when retained, in which case
/// its blocks must not be freed.
pub fn retire(
    snapshots: &[Lsn],
    retained: &mut Vec<IndexEntry>,
    old: &IndexEntry,
    new_lsn: Lsn,
) -> bool {
    if old.flags & (entry_flags::IS_DIR | entry_flags::IS_DELETED) != 0
        || !pins(snapshots, old.lsn, new_lsn)
    {
        return false;
    }
    let mut kept = *old;
    kept._reserved[..8].copy_from_slice(&new_lsn.to_le_bytes());
    retained.push(kept);
    true
}

/// The retained version of `path` that was current at `lsn`.
pub fn version_at<'a>(retained: &'a [IndexEntry], path: &str, lsn: Lsn) -> Option<&'a IndexEntry> {
    retained
        .iter()
        .find(|e| path_bytes(&e.path) == path.as_bytes() && e.lsn <= lsn && lsn < superseded_lsn(e))
}

pub fn snapshot_mark(lsn: Lsn) -> IndexEntry {
    // SAFETY: all-zeros is valid for every IndexEntry field.
    let mut e: IndexEntry = unsafe { core::mem::zeroed() };
    e.flags = entry_flags::IS_SNAPSHOT_MARK;
    e.lsn = lsn;
    e
}
