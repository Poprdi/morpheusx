//! Orchestrates ISO storage: GPT partitioning, manifests on ESP, chunk I/O.

use super::chunk::{ChunkSet, MAX_CHUNKS};
use super::error::IsoError;
use super::manifest::IsoManifest;
use super::reader::{ChunkReader, IsoReadContext};
use super::writer::ChunkWriter;
use super::{DEFAULT_CHUNK_SIZE, FAT32_MAX_FILE_SIZE};

pub const MAX_ISOS: usize = 8;

pub const MANIFEST_DIR: &str = "/.iso";

/// ISO metadata, no chunk data.
#[derive(Clone)]
pub struct IsoEntry {
    pub manifest: IsoManifest,
    pub valid: bool,
}

impl IsoEntry {
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

#[derive(Debug, Clone)]
pub struct AllocationResult {
    /// Per-chunk partition LBAs (start, end).
    pub partitions: [(u64, u64); MAX_CHUNKS],
    pub count: usize,
    pub total_bytes: u64,
}

/// Tracks ISO manifests and coordinates chunk allocation.
pub struct IsoStorageManager {
    entries: [IsoEntry; MAX_ISOS],
    entry_count: usize,
    /// ESP partition start LBA for manifest storage.
    esp_start_lba: u64,
    target_disk_size_lba: u64,
    chunk_size: u64,
}

impl IsoStorageManager {
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

    pub fn set_chunk_size(&mut self, size: u64) {
        self.chunk_size = size.min(FAT32_MAX_FILE_SIZE);
    }

    pub fn count(&self) -> usize {
        self.entry_count
    }

    pub fn get(&self, index: usize) -> Option<&IsoEntry> {
        if index < self.entry_count && self.entries[index].valid {
            Some(&self.entries[index])
        } else {
            None
        }
    }

    pub fn find_by_name(&self, name: &str) -> Option<usize> {
        (0..self.entry_count)
            .find(|&i| self.entries[i].valid && self.entries[i].manifest.name_str() == name)
    }

    pub fn chunks_needed(&self, iso_size: u64) -> usize {
        ((iso_size + self.chunk_size - 1) / self.chunk_size) as usize
    }

    /// Disk space needed including ~1 MB/chunk FAT32 overhead.
    pub fn space_needed(&self, iso_size: u64) -> u64 {
        let chunks = self.chunks_needed(iso_size);
        iso_size + (chunks as u64 * 1024 * 1024)
    }

    /// Rough estimate; assumes half the disk is usable.
    pub fn has_space_for(&self, iso_size: u64) -> bool {
        let needed_lba = self.space_needed(iso_size) / 512;
        needed_lba < self.target_disk_size_lba / 2
    }

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

    pub fn remove_entry(&mut self, index: usize) -> Result<(), IsoError> {
        if index >= self.entry_count || !self.entries[index].valid {
            return Err(IsoError::ManifestNotFound);
        }

        for i in index..self.entry_count - 1 {
            self.entries[i] = self.entries[i + 1].clone();
        }
        self.entries[self.entry_count - 1] = IsoEntry::empty();
        self.entry_count -= 1;

        Ok(())
    }

    /// Creates the manifest and reserves slots; partitions created via GPT ops.
    /// Returns (entry_index, manifest) for a ChunkWriter.
    pub fn prepare_download(
        &mut self,
        name: &str,
        total_size: u64,
        sha256: Option<&[u8; 32]>,
    ) -> Result<(usize, IsoManifest), IsoError> {
        if self.find_by_name(name).is_some() {
            return Err(IsoError::ManifestExists);
        }

        let chunks_needed = self.chunks_needed(total_size);
        if chunks_needed > MAX_CHUNKS {
            return Err(IsoError::IsoTooLarge);
        }

        let mut manifest = IsoManifest::new(name, total_size);
        if let Some(hash) = sha256 {
            manifest.set_sha256(hash);
        }

        Ok((self.entry_count, manifest))
    }

    pub fn finalize_download(
        &mut self,
        mut manifest: IsoManifest,
        chunks: ChunkSet,
    ) -> Result<usize, IsoError> {
        manifest.chunks = chunks;
        manifest.mark_complete();
        self.add_entry(manifest)
    }

    pub fn create_writer(&self, manifest: &IsoManifest) -> Result<ChunkWriter, IsoError> {
        ChunkWriter::from_manifest(manifest)
    }

    pub fn create_reader(&self, index: usize) -> Result<ChunkReader, IsoError> {
        let entry = self.get(index).ok_or(IsoError::ManifestNotFound)?;

        if !entry.manifest.is_complete() {
            return Err(IsoError::DataCorruption);
        }

        ChunkReader::from_manifest(&entry.manifest)
    }

    pub fn get_read_context(&self, index: usize) -> Result<IsoReadContext, IsoError> {
        let entry = self.get(index).ok_or(IsoError::ManifestNotFound)?;

        if !entry.manifest.is_complete() {
            return Err(IsoError::DataCorruption);
        }

        Ok(IsoReadContext::from_manifest(&entry.manifest))
    }

    pub fn iter(&self) -> IsoEntryIterator<'_> {
        IsoEntryIterator {
            entries: &self.entries,
            count: self.entry_count,
            index: 0,
        }
    }

    /// Partition LBAs used by all ISOs, for avoiding allocation conflicts.
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

    pub fn esp_start_lba(&self) -> u64 {
        self.esp_start_lba
    }

    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }
}

impl Default for IsoStorageManager {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

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

#[derive(Debug, Clone, Copy)]
pub struct PartitionRequest {
    pub min_size: u64,
    pub preferred_size: u64,
    /// GPT partition name.
    pub name: [u8; 32],
    pub name_len: usize,
}

impl PartitionRequest {
    pub fn for_chunk(chunk_index: usize, data_size: u64) -> Self {
        let mut name = [0u8; 32];
        let prefix = b"ISO_CHUNK_";
        name[..prefix.len()].copy_from_slice(prefix);
        let digit = b'0' + (chunk_index as u8 % 10);
        name[prefix.len()] = digit;

        Self {
            min_size: data_size,
            preferred_size: data_size + (1024 * 1024), // +1 MB FAT32 overhead
            name,
            name_len: prefix.len() + 1,
        }
    }

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

        assert_eq!(manager.chunks_needed(1_000_000_000), 1);
        assert_eq!(manager.chunks_needed(DEFAULT_CHUNK_SIZE), 1);
        assert_eq!(manager.chunks_needed(DEFAULT_CHUNK_SIZE + 1), 2);
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

        let manifest = IsoManifest::new("ubuntu.iso", 1_000_000_000);
        manager.add_entry(manifest).unwrap();

        let result = manager.prepare_download("ubuntu.iso", 1_000_000_000, None);
        assert!(matches!(result, Err(IsoError::ManifestExists)));
    }
}
