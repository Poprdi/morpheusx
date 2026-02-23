//! In-memory B-tree for the namespace index.
//!
//! This is a sorted-array B-tree stored entirely in `Vec<IndexEntry>`.
//! On-disk serialization (checkpoint) is handled externally.
//!
//! Operations:
//! - `lookup(path_hash, path)` → `Option<&IndexEntry>`
//! - `insert(entry)` — upsert by path_hash + path
//! - `delete(path_hash, path)` — marks entry as deleted
//! - `prefix_scan(parent_path)` → iterator of direct children (readdir)
//! - `all_entries()` → iterator for checkpoint serialization

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum entries before we should warn (not a hard limit).
const EXPECTED_MAX_ENTRIES: usize = 1_000_000;

/// In-memory namespace index.
///
/// Simple sorted-vec implementation for correctness.  A proper B-tree
/// node structure can replace this later for O(log n) operations on
/// very large namespaces.  For typical use (< 100K files), binary
/// search on a sorted vec is fast enough and much simpler to verify.
pub struct NamespaceIndex {
    /// Sorted by (key, path bytes).
    entries: Vec<IndexEntry>,
}

impl Default for NamespaceIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(1024),
        }
    }

    /// Number of live (non-deleted) entries.
    pub fn live_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.flags & entry_flags::IS_DELETED == 0)
            .count()
    }

    /// Total entries including deleted tombstones.
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }

    /// Find the position of an entry by key + path.
    fn find_pos(&self, key: u64, path: &[u8]) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| {
            e.key.cmp(&key).then_with(|| {
                let e_path = path_bytes(&e.path);
                e_path.cmp(path)
            })
        })
    }

    /// Look up an entry by exact path string.
    pub fn lookup(&self, path: &str) -> Option<&IndexEntry> {
        let key = fnv1a_64(path.as_bytes());
        let path_b = path.as_bytes();
        match self.find_pos(key, path_b) {
            Ok(idx) => {
                let entry = &self.entries[idx];
                if entry.flags & entry_flags::IS_DELETED != 0 {
                    None
                } else {
                    Some(entry)
                }
            }
            Err(_) => None,
        }
    }

    /// Look up an entry by path, trying both with and without trailing `/`.
    ///
    /// Directories are stored with a trailing `/` (e.g. `/home/`), but user
    /// paths often omit it (e.g. `/home`).  This method bridges the gap.
    pub fn lookup_flex(&self, path: &str) -> Option<&IndexEntry> {
        // Try exact match first.
        if let Some(entry) = self.lookup(path) {
            return Some(entry);
        }
        // Try the alternate form.
        if !path.ends_with('/') {
            let mut with_slash = String::from(path);
            with_slash.push('/');
            self.lookup(&with_slash)
        } else if path.len() > 1 {
            self.lookup(&path[..path.len() - 1])
        } else {
            None
        }
    }

    /// Look up a mutable reference by exact path string.
    pub fn lookup_mut(&mut self, path: &str) -> Option<&mut IndexEntry> {
        let key = fnv1a_64(path.as_bytes());
        let path_b = path.as_bytes();
        match self.find_pos(key, path_b) {
            Ok(idx) => {
                let entry = &mut self.entries[idx];
                if entry.flags & entry_flags::IS_DELETED != 0 {
                    None
                } else {
                    Some(entry)
                }
            }
            Err(_) => None,
        }
    }

    /// Mutable flexible lookup — tries both with and without trailing `/`.
    pub fn lookup_flex_mut(&mut self, path: &str) -> Option<&mut IndexEntry> {
        // Try exact match first.
        if self.lookup(path).is_some() {
            return self.lookup_mut(path);
        }
        // Try alternate form.
        if !path.ends_with('/') {
            let mut with_slash = String::from(path);
            with_slash.push('/');
            if self.lookup(&with_slash).is_some() {
                return self.lookup_mut(&with_slash);
            }
        } else if path.len() > 1 {
            let without = &path[..path.len() - 1];
            if self.lookup(without).is_some() {
                return self.lookup_mut(without);
            }
        }
        None
    }

    /// Look up an entry by key (hash) alone.
    ///
    /// This is used by the VFS layer when we have an fd with only the
    /// key stored.  If multiple entries share the same key (hash collision),
    /// the first live match is returned.
    pub fn lookup_by_key(&self, key: u64) -> Option<&IndexEntry> {
        // Binary search for the first entry with this key.
        let start = self.entries.partition_point(|e| e.key < key);
        for entry in &self.entries[start..] {
            if entry.key != key {
                break;
            }
            if entry.flags & entry_flags::IS_DELETED == 0 {
                return Some(entry);
            }
        }
        None
    }

    /// Insert or update an entry.
    pub fn upsert(&mut self, entry: IndexEntry) {
        let path_b = path_bytes(&entry.path);
        match self.find_pos(entry.key, path_b) {
            Ok(idx) => {
                // Update in place.
                self.entries[idx] = entry;
            }
            Err(idx) => {
                // Insert at sorted position.
                self.entries.insert(idx, entry);
            }
        }
    }

    /// Mark an entry as deleted (tombstone).
    pub fn mark_deleted(&mut self, path: &str) -> Result<(), HelixError> {
        // Try exact path first, then alternate form (with/without trailing '/').
        let idx = {
            let key = fnv1a_64(path.as_bytes());
            match self.find_pos(key, path.as_bytes()) {
                Ok(i) => i,
                Err(_) => {
                    // Try alternate form.
                    let alt = if !path.ends_with('/') {
                        let mut s = String::from(path);
                        s.push('/');
                        s
                    } else if path.len() > 1 {
                        String::from(&path[..path.len() - 1])
                    } else {
                        return Err(HelixError::NotFound);
                    };
                    let alt_key = fnv1a_64(alt.as_bytes());
                    self.find_pos(alt_key, alt.as_bytes())
                        .map_err(|_| HelixError::NotFound)?
                }
            }
        };
        self.entries[idx].flags |= entry_flags::IS_DELETED;
        Ok(())
    }

    /// List direct children of a directory path.
    ///
    /// `parent_path` should end with `/` for directories, or be `"/"`
    /// for root.  Returns entries whose paths match `parent_path + name`
    /// with no further `/` separators.
    pub fn readdir(&self, parent_path: &str) -> Vec<&IndexEntry> {
        let prefix = if parent_path == "/" {
            "/".as_bytes()
        } else {
            parent_path.as_bytes()
        };

        let mut results = Vec::new();

        for entry in &self.entries {
            if entry.flags & entry_flags::IS_DELETED != 0 {
                continue;
            }
            let e_path = path_bytes(&entry.path);
            // Must start with parent path.
            if !e_path.starts_with(prefix) {
                continue;
            }
            // Skip the parent itself.
            if e_path == prefix {
                continue;
            }
            // For root "/", children are like "/foo" or "/bar/".
            // For "/data/", children are like "/data/foo" or "/data/bar/".
            let remainder = &e_path[prefix.len()..];
            // Strip leading '/' if parent is root.
            let remainder = if parent_path == "/" && remainder.starts_with(b"/") {
                // Actually, for root, prefix is "/" so e_path "/foo" has
                // remainder "foo".  But "/data/docs" has remainder "data/docs".
                // We want only entries with no '/' in the remainder.
                remainder
            } else {
                remainder
            };

            // Direct child: no '/' in remainder, or remainder ends with '/'
            // and has no other '/'.
            let slash_count = remainder.iter().filter(|&&b| b == b'/').count();
            let is_direct_child = slash_count == 0
                || (slash_count == 1 && remainder.last() == Some(&b'/'));

            if is_direct_child {
                results.push(entry);
            }
        }

        results
    }

    /// Get all live entries (for checkpoint serialization).
    pub fn all_entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    /// Get all live entries as a mutable reference.
    pub fn all_entries_mut(&mut self) -> &mut [IndexEntry] {
        &mut self.entries
    }

    /// Remove all tombstoned entries from the in-memory index.
    pub fn compact(&mut self) {
        self.entries
            .retain(|e| e.flags & entry_flags::IS_DELETED == 0);
    }

    /// Clear the entire index (used during recovery before replay).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Build a new IndexEntry for a file.
    pub fn make_file_entry(
        path: &str,
        lsn: Lsn,
        size: u64,
        timestamp_ns: u64,
        inline_data: Option<&[u8]>,
        extent_root: BlockAddr,
        content_crc64: u64,
    ) -> IndexEntry {
        let mut entry: IndexEntry = unsafe { core::mem::zeroed() };
        entry.key = fnv1a_64(path.as_bytes());
        set_path(&mut entry.path, path.as_bytes());
        entry.lsn = lsn;
        entry.size = size;
        entry.created_ns = timestamp_ns;
        entry.modified_ns = timestamp_ns;
        entry.version_count = 1;
        entry.first_lsn = lsn;
        entry.content_crc64 = content_crc64;

        if let Some(data) = inline_data {
            if data.len() <= INLINE_DATA_SIZE {
                entry.flags |= entry_flags::IS_INLINE;
                entry.inline_data[..data.len()].copy_from_slice(data);
                entry.extent_root = BLOCK_NULL;
            } else {
                entry.extent_root = extent_root;
            }
        } else {
            entry.extent_root = extent_root;
        }

        entry
    }

    /// Build a new IndexEntry for a directory.
    pub fn make_dir_entry(
        path: &str,
        lsn: Lsn,
        timestamp_ns: u64,
    ) -> IndexEntry {
        let mut entry: IndexEntry = unsafe { core::mem::zeroed() };
        entry.key = fnv1a_64(path.as_bytes());
        set_path(&mut entry.path, path.as_bytes());
        entry.flags = entry_flags::IS_DIR;
        entry.lsn = lsn;
        entry.created_ns = timestamp_ns;
        entry.modified_ns = timestamp_ns;
        entry.version_count = 1;
        entry.first_lsn = lsn;
        entry
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Extract the null-terminated path from a fixed-size buffer.
pub fn path_bytes(buf: &[u8; 256]) -> &[u8] {
    let len = buf.iter().position(|&b| b == 0).unwrap_or(256);
    &buf[..len]
}

/// Extract the path as a `&str`.
pub fn path_str(buf: &[u8; 256]) -> &str {
    let bytes = path_bytes(buf);
    core::str::from_utf8(bytes).unwrap_or("")
}

/// Write path bytes into a fixed buffer.
fn set_path(buf: &mut [u8; 256], path: &[u8]) {
    let len = path.len().min(MAX_PATH_LEN);
    buf[..len].copy_from_slice(&path[..len]);
    if len < 256 {
        buf[len] = 0;
    }
}

/// Extract the filename (last path component) from a full path.
pub fn filename(path: &str) -> &str {
    if path == "/" {
        return "/";
    }
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

/// Extract the parent directory from a full path.
pub fn parent_path(path: &str) -> &str {
    if path == "/" {
        return "/";
    }
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(0) => "/",
        Some(idx) => &path[..idx + 1],
        None => "/",
    }
}
