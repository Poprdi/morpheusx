//! In-memory namespace index. Sorted `Vec<IndexEntry>`; on-disk serialization via checkpoint.

use crate::crc::fnv1a_64;
use crate::error::HelixError;
use crate::types::*;
use alloc::string::String;
use alloc::vec::Vec;

const EXPECTED_MAX_ENTRIES: usize = 1_000_000;

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
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(1024),
        }
    }

    pub fn live_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.flags & entry_flags::IS_DELETED == 0)
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.entries.len()
    }

    fn find_pos(&self, key: u64, path: &[u8]) -> Result<usize, usize> {
        self.entries.binary_search_by(|e| {
            e.key.cmp(&key).then_with(|| {
                let e_path = path_bytes(&e.path);
                e_path.cmp(path)
            })
        })
    }

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
            },
            Err(_) => None,
        }
    }

    /// Try path as-is, then toggle trailing `/`. Dirs are stored with trailing `/`.
    pub fn lookup_flex(&self, path: &str) -> Option<&IndexEntry> {
        if let Some(entry) = self.lookup(path) {
            return Some(entry);
        }
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
            },
            Err(_) => None,
        }
    }

    pub fn lookup_flex_mut(&mut self, path: &str) -> Option<&mut IndexEntry> {
        if self.lookup(path).is_some() {
            return self.lookup_mut(path);
        }
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

    /// Look up by key (hash) alone. On collision returns the first live match.
    pub fn lookup_by_key(&self, key: u64) -> Option<&IndexEntry> {
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

    /// Upsert; auto-compact tombstones once they exceed live entries (and index >= 512).
    pub fn upsert(&mut self, entry: IndexEntry) {
        let path_b = path_bytes(&entry.path);
        match self.find_pos(entry.key, path_b) {
            Ok(idx) => {
                self.entries[idx] = entry;
            },
            Err(idx) => {
                self.entries.insert(idx, entry);
            },
        }

        let total = self.entries.len();
        let live = self.live_count();
        if total >= 512 && total > live.saturating_mul(2) {
            self.compact();
        }
    }

    pub fn mark_deleted(&mut self, path: &str) -> Result<(), HelixError> {
        let idx = {
            let key = fnv1a_64(path.as_bytes());
            match self.find_pos(key, path.as_bytes()) {
                Ok(i) => i,
                Err(_) => {
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
                },
            }
        };
        self.entries[idx].flags |= entry_flags::IS_DELETED;
        Ok(())
    }

    /// Direct children of `parent_path`. Use `"/"` for root; otherwise must end with `/`.
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
            if !e_path.starts_with(prefix) {
                continue;
            }
            if e_path == prefix {
                continue;
            }
            let remainder = &e_path[prefix.len()..];

            // Direct child: no '/' in remainder, or trailing '/' only.
            let slash_count = remainder.iter().filter(|&&b| b == b'/').count();
            let is_direct_child =
                slash_count == 0 || (slash_count == 1 && remainder.last() == Some(&b'/'));

            if is_direct_child {
                results.push(entry);
            }
        }

        results
    }

    pub fn all_entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    pub fn all_entries_mut(&mut self) -> &mut [IndexEntry] {
        &mut self.entries
    }

    /// Drop tombstones from the in-memory index.
    pub fn compact(&mut self) {
        self.entries
            .retain(|e| e.flags & entry_flags::IS_DELETED == 0);
    }

    /// Used during recovery before replay.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

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

    pub fn make_dir_entry(path: &str, lsn: Lsn, timestamp_ns: u64) -> IndexEntry {
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

pub fn path_bytes(buf: &[u8; 256]) -> &[u8] {
    let len = buf.iter().position(|&b| b == 0).unwrap_or(256);
    &buf[..len]
}

pub fn path_str(buf: &[u8; 256]) -> &str {
    let bytes = path_bytes(buf);
    core::str::from_utf8(bytes).unwrap_or("")
}

fn set_path(buf: &mut [u8; 256], path: &[u8]) {
    let len = path.len().min(MAX_PATH_LEN);
    buf[..len].copy_from_slice(&path[..len]);
    if len < 256 {
        buf[len] = 0;
    }
}

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
