//! Chunk information structures
//!
//! Fixed-size structures for tracking ISO chunks across partitions.
//! All structures use fixed arrays to avoid heap allocation where possible.

/// Maximum number of chunks per ISO (16 * 4GB = 64GB max ISO size)
pub const MAX_CHUNKS: usize = 16;

/// Maximum filename length (8.3 format compatible)
pub const MAX_FILENAME_LEN: usize = 12;

/// Information about a single chunk partition
#[derive(Debug, Clone, Copy)]
pub struct ChunkInfo {
    /// Partition UUID (16 bytes, from GPT)
    pub partition_uuid: [u8; 16],
    /// Start LBA of the partition
    pub start_lba: u64,
    /// End LBA of the partition
    pub end_lba: u64,
    /// Size of data stored in this chunk (bytes)
    pub data_size: u64,
    /// Chunk index (0-based)
    pub index: u8,
    /// Whether this chunk has been written
    pub written: bool,
}

impl ChunkInfo {
    /// Create an empty/uninitialized chunk info
    pub const fn empty() -> Self {
        Self {
            partition_uuid: [0u8; 16],
            start_lba: 0,
            end_lba: 0,
            data_size: 0,
            index: 0,
            written: false,
        }
    }

    /// Check if this chunk info is valid (has a partition assigned)
    pub const fn is_valid(&self) -> bool {
        self.start_lba != 0 && self.end_lba > self.start_lba
    }

    /// Get partition size in bytes (assuming 512-byte sectors)
    pub const fn partition_size(&self) -> u64 {
        (self.end_lba - self.start_lba + 1) * 512
    }

    /// Create a new chunk info for a partition
    pub const fn new(partition_uuid: [u8; 16], start_lba: u64, end_lba: u64, index: u8) -> Self {
        Self {
            partition_uuid,
            start_lba,
            end_lba,
            data_size: 0,
            index,
            written: false,
        }
    }
}

/// Collection of chunks for a single ISO
#[derive(Debug, Clone)]
pub struct ChunkSet {
    /// Array of chunk info (fixed size, use count for valid entries)
    pub chunks: [ChunkInfo; MAX_CHUNKS],
    /// Number of valid chunks in the array
    pub count: usize,
    /// Total ISO size in bytes
    pub total_size: u64,
    /// Bytes written so far (for progress tracking)
    pub bytes_written: u64,
}

impl ChunkSet {
    /// Create a new empty chunk set
    pub const fn new() -> Self {
        Self {
            chunks: [ChunkInfo::empty(); MAX_CHUNKS],
            count: 0,
            total_size: 0,
            bytes_written: 0,
        }
    }

    /// Add a chunk to the set
    ///
    /// Returns the chunk index on success, or None if set is full
    pub fn add_chunk(&mut self, info: ChunkInfo) -> Option<usize> {
        if self.count >= MAX_CHUNKS {
            return None;
        }
        let idx = self.count;
        self.chunks[idx] = info;
        self.count += 1;
        Some(idx)
    }

    /// Get a chunk by index
    pub fn get(&self, index: usize) -> Option<&ChunkInfo> {
        if index < self.count {
            Some(&self.chunks[index])
        } else {
            None
        }
    }

    /// Get a mutable chunk by index
    pub fn get_mut(&mut self, index: usize) -> Option<&mut ChunkInfo> {
        if index < self.count {
            Some(&mut self.chunks[index])
        } else {
            None
        }
    }

    /// Find chunk containing a given byte offset
    pub fn chunk_for_offset(&self, offset: u64) -> Option<(usize, u64)> {
        let mut cumulative = 0u64;
        for i in 0..self.count {
            let chunk_size = self.chunks[i].data_size;
            if offset < cumulative + chunk_size {
                return Some((i, offset - cumulative));
            }
            cumulative += chunk_size;
        }
        None
    }

    /// Calculate total capacity of all chunks
    pub fn total_capacity(&self) -> u64 {
        let mut total = 0u64;
        for i in 0..self.count {
            total += self.chunks[i].partition_size();
        }
        total
    }

    /// Check if all chunks have been written
    pub fn is_complete(&self) -> bool {
        if self.count == 0 {
            return false;
        }
        for i in 0..self.count {
            if !self.chunks[i].written {
                return false;
            }
        }
        true
    }

    /// Get write progress as percentage (0-100)
    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        ((self.bytes_written * 100) / self.total_size) as u8
    }

    /// Iterator over valid chunks
    pub fn iter(&self) -> ChunkIterator<'_> {
        ChunkIterator {
            chunks: &self.chunks,
            count: self.count,
            index: 0,
        }
    }
}

impl Default for ChunkSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over chunks in a ChunkSet
pub struct ChunkIterator<'a> {
    chunks: &'a [ChunkInfo; MAX_CHUNKS],
    count: usize,
    index: usize,
}

impl<'a> Iterator for ChunkIterator<'a> {
    type Item = &'a ChunkInfo;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.count {
            let chunk = &self.chunks[self.index];
            self.index += 1;
            Some(chunk)
        } else {
            None
        }
    }
}
