//! ISO Storage Manager
//!
//! High-level orchestration for ISO storage operations. This module
//! coordinates GPT partitioning, manifest management, and chunk I/O.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    IsoStorageManager                        │
//! ├─────────────────────────────────────────────────────────────┤
//! │  - Enumerate stored ISOs (from manifests on ESP)            │
//! │  - Allocate chunk partitions for new ISOs                   │
//! │  - Track download progress                                  │
//! │  - Provide read access for booting                          │
//! └─────────────────────────────────────────────────────────────┘
//!          │                    │                    │
//!          ▼                    ▼                    ▼
//!   ┌────────────┐      ┌────────────┐      ┌────────────┐
//!   │  manifest  │      │   writer   │      │   reader   │
//!   │  (ESP)     │      │  (chunks)  │      │  (chunks)  │
//!   └────────────┘      └────────────┘      └────────────┘
//! ```

use super::chunk::{ChunkSet, MAX_CHUNKS};
use super::error::IsoError;
use super::manifest::IsoManifest;
use super::reader::{ChunkReader, IsoReadContext};
use super::writer::ChunkWriter;
use super::{DEFAULT_CHUNK_SIZE, FAT32_MAX_FILE_SIZE};

/// Maximum number of ISOs that can be tracked
pub const MAX_ISOS: usize = 8;

/// Manifest directory path on ESP
pub const MANIFEST_DIR: &str = "/morpheus/isos";

/// ISO storage entry (metadata only, no chunk data)
#[derive(Clone)]
pub struct IsoEntry {
    /// Manifest data
    pub manifest: IsoManifest,
    /// Whether this entry is valid/populated
    pub valid: bool,
}

impl IsoEntry {
    /// Create an empty entry
    pub const fn empty() -> Self {
        Self {
            manifest: IsoManifest {
                name: [0u8; 64],
                name_len: 0,
                total_size: 0,
                sha256: [0u8; 32],
                chunks: ChunkSet::new(),
                flags: 0,
            },
            valid: false,
        }
    }
}

impl Default for IsoEntry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Storage allocation result
#[derive(Debug, Clone)]
pub struct AllocationResult {
    /// Partition LBAs (start, end) for each chunk
    pub partitions: [(u64, u64); MAX_CHUNKS],
    /// Number of partitions allocated
    pub count: usize,
    /// Total space allocated in bytes
    pub total_bytes: u64,
}

/// ISO storage manager
///
/// Manages ISO manifests and coordinates chunk allocation.
pub struct IsoStorageManager {
    /// Cached ISO entries (loaded from ESP)
    entries: [IsoEntry; MAX_ISOS],
    /// Number of valid entries
    entry_count: usize,
    /// ESP partition start LBA (for manifest storage)
    esp_start_lba: u64,
    /// Target disk for chunk partitions
    target_disk_size_lba: u64,
    /// Chunk size to use
    chunk_size: u64,
}

impl IsoStorageManager {
    /// Create a new storage manager
    pub fn new(esp_start_lba: u64, target_disk_size_lba: u64) -> Self {
        Self {
            entries: [
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
                IsoEntry::empty(),
            ],
            entry_count: 0,
            esp_start_lba,
            target_disk_size_lba,
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Set custom chunk size (must be <= FAT32_MAX_FILE_SIZE)
    pub fn set_chunk_size(&mut self, size: u64) {
        self.chunk_size = size.min(FAT32_MAX_FILE_SIZE);
    }

    /// Get number of stored ISOs
    pub fn count(&self) -> usize {
        self.entry_count
    }

    /// Get ISO entry by index
    pub fn get(&self, index: usize) -> Option<&IsoEntry> {
        if index < self.entry_count && self.entries[index].valid {
            Some(&self.entries[index])
        } else {
            None
        }
    }

    /// Find ISO by name
    pub fn find_by_name(&self, name: &str) -> Option<usize> {
        for i in 0..self.entry_count {
            if self.entries[i].valid && self.entries[i].manifest.name_str() == name {
                return Some(i);
            }
        }
        None
    }

    /// Calculate chunks needed for an ISO
    pub fn chunks_needed(&self, iso_size: u64) -> usize {
        ((iso_size + self.chunk_size - 1) / self.chunk_size) as usize
    }

    /// Calculate total disk space needed (with FAT32 overhead)
    pub fn space_needed(&self, iso_size: u64) -> u64 {
        let chunks = self.chunks_needed(iso_size);
        // Add ~1MB per chunk for FAT32 overhead
        iso_size + (chunks as u64 * 1024 * 1024)
    }

    /// Check if there's enough space for an ISO
    ///
    /// This is a quick estimate - actual allocation may vary based on
    /// partition alignment and existing partitions.
    pub fn has_space_for(&self, iso_size: u64) -> bool {
        let needed_lba = self.space_needed(iso_size) / 512;
        // Very rough estimate - assumes half the disk is usable
        needed_lba < self.target_disk_size_lba / 2
    }

    /// Add a manifest entry (called after loading from ESP)
    pub fn add_entry(&mut self, manifest: IsoManifest) -> Result<usize, IsoError> {
        if self.entry_count >= MAX_ISOS {
            return Err(IsoError::IsoTooLarge);
        }

        let index = self.entry_count;
        self.entries[index] = IsoEntry {
            manifest,
            valid: true,
        };
        self.entry_count += 1;

        Ok(index)
    }

    /// Remove an ISO entry by index
    pub fn remove_entry(&mut self, index: usize) -> Result<(), IsoError> {
        if index >= self.entry_count || !self.entries[index].valid {
            return Err(IsoError::ManifestNotFound);
        }

        // Shift entries down
        for i in index..self.entry_count - 1 {
            self.entries[i] = self.entries[i + 1].clone();
        }
        self.entries[self.entry_count - 1] = IsoEntry::empty();
        self.entry_count -= 1;

        Ok(())
    }

    /// Prepare storage for a new ISO download
    ///
    /// This creates the manifest and reserves partition slots.
    /// The actual partitions should be created via GPT operations.
    ///
    /// Returns (entry_index, manifest) that can be used with a ChunkWriter.
    pub fn prepare_download(
        &mut self,
        name: &str,
        total_size: u64,
        sha256: Option<&[u8; 32]>,
    ) -> Result<(usize, IsoManifest), IsoError> {
        // Check if already exists
        if self.find_by_name(name).is_some() {
            return Err(IsoError::ManifestExists);
        }

        // Check space
        let chunks_needed = self.chunks_needed(total_size);
        if chunks_needed > MAX_CHUNKS {
            return Err(IsoError::IsoTooLarge);
        }

        // Create manifest
        let mut manifest = IsoManifest::new(name, total_size);
        if let Some(hash) = sha256 {
            manifest.set_sha256(hash);
        }

        Ok((self.entry_count, manifest))
    }

    /// Finalize a download (update manifest, add entry)
    pub fn finalize_download(
        &mut self,
        mut manifest: IsoManifest,
        chunks: ChunkSet,
    ) -> Result<usize, IsoError> {
        // Update manifest with actual chunk data
        manifest.chunks = chunks;
        manifest.mark_complete();

        // Add to entries
        self.add_entry(manifest)
    }

    /// Create a ChunkWriter for an ISO download
    pub fn create_writer(&self, manifest: &IsoManifest) -> Result<ChunkWriter, IsoError> {
        ChunkWriter::from_manifest(manifest)
    }

    /// Create a ChunkReader for booting an ISO
    pub fn create_reader(&self, index: usize) -> Result<ChunkReader, IsoError> {
        let entry = self.get(index).ok_or(IsoError::ManifestNotFound)?;

        if !entry.manifest.is_complete() {
            return Err(IsoError::DataCorruption);
        }

        ChunkReader::from_manifest(&entry.manifest)
    }

    /// Get read context for boot loader (lightweight, copyable)
    pub fn get_read_context(&self, index: usize) -> Result<IsoReadContext, IsoError> {
        let entry = self.get(index).ok_or(IsoError::ManifestNotFound)?;

        if !entry.manifest.is_complete() {
            return Err(IsoError::DataCorruption);
        }

        Ok(IsoReadContext::from_manifest(&entry.manifest))
    }

    /// Iterator over valid ISO entries
    pub fn iter(&self) -> IsoEntryIterator<'_> {
        IsoEntryIterator {
            entries: &self.entries,
            count: self.entry_count,
            index: 0,
        }
    }

    /// Get list of partition LBAs used by all ISOs
    ///
    /// Useful for avoiding allocation conflicts when creating new partitions.
    pub fn used_partitions(&self) -> [(u64, u64); 64] {
        let mut result = [(0u64, 0u64); 64];
        let mut idx = 0;

        for i in 0..self.entry_count {
            if !self.entries[i].valid {
                continue;
            }

            for chunk in self.entries[i].manifest.chunks.iter() {
                if idx < 64 && chunk.is_valid() {
                    result[idx] = (chunk.start_lba, chunk.end_lba);
                    idx += 1;
                }
            }
        }

        result
    }

    /// Get ESP start LBA
    pub fn esp_start_lba(&self) -> u64 {
        self.esp_start_lba
    }

    /// Get chunk size
    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }
}

impl Default for IsoStorageManager {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// Iterator over ISO entries
pub struct IsoEntryIterator<'a> {
    entries: &'a [IsoEntry; MAX_ISOS],
    count: usize,
    index: usize,
}

impl<'a> Iterator for IsoEntryIterator<'a> {
    type Item = (usize, &'a IsoEntry);

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.count {
            let i = self.index;
            self.index += 1;

            if self.entries[i].valid {
                return Some((i, &self.entries[i]));
            }
        }
        None
    }
}

/// Partition allocation request
#[derive(Debug, Clone, Copy)]
pub struct PartitionRequest {
    /// Minimum size in bytes
    pub min_size: u64,
    /// Preferred size in bytes
    pub preferred_size: u64,
    /// Partition name (for GPT)
    pub name: [u8; 32],
    /// Name length
    pub name_len: usize,
}

impl PartitionRequest {
    /// Create a request for a chunk partition
    pub fn for_chunk(chunk_index: usize, data_size: u64) -> Self {
        let mut name = [0u8; 32];
        let prefix = b"ISO_CHUNK_";
        name[..prefix.len()].copy_from_slice(prefix);
        // Add chunk number
        let digit = b'0' + (chunk_index as u8 % 10);
        name[prefix.len()] = digit;

        Self {
            min_size: data_size,
            preferred_size: data_size + (1024 * 1024), // +1MB for FAT32 overhead
            name,
            name_len: prefix.len() + 1,
        }
    }

    /// Get partition name as string
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("ISO_CHUNK")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunks_needed() {
        let manager = IsoStorageManager::new(0, 100_000_000);

        // Under 4GB - 1 chunk
        assert_eq!(manager.chunks_needed(1_000_000_000), 1);

        // Exactly 4GB - 1 chunk
        assert_eq!(manager.chunks_needed(DEFAULT_CHUNK_SIZE), 1);

        // Just over 4GB - 2 chunks
        assert_eq!(manager.chunks_needed(DEFAULT_CHUNK_SIZE + 1), 2);

        // ~8GB - 2 chunks
        assert_eq!(manager.chunks_needed(8_000_000_000), 2);
    }

    #[test]
    fn test_prepare_download() {
        let mut manager = IsoStorageManager::new(2048, 100_000_000);

        let (idx, manifest) = manager
            .prepare_download("ubuntu.iso", 5_000_000_000, None)
            .unwrap();

        assert_eq!(idx, 0);
        assert_eq!(manifest.name_str(), "ubuntu.iso");
        assert_eq!(manifest.total_size, 5_000_000_000);
    }

    #[test]
    fn test_duplicate_detection() {
        let mut manager = IsoStorageManager::new(2048, 100_000_000);

        // Add first ISO
        let manifest = IsoManifest::new("ubuntu.iso", 1_000_000_000);
        manager.add_entry(manifest).unwrap();

        // Try to prepare duplicate
        let result = manager.prepare_download("ubuntu.iso", 1_000_000_000, None);
        assert!(matches!(result, Err(IsoError::ManifestExists)));
    }
}
