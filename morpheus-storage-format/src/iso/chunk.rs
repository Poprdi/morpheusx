//! Fixed-array chunk tracking for ISOs split across partitions; heap-free.

/// 16 * 4 GB = 64 GB max ISO.
pub const MAX_CHUNKS: usize = 16;

pub const MAX_FILENAME_LEN: usize = 12;

#[derive(Debug, Clone, Copy)]
pub struct ChunkInfo {
    /// GPT partition UUID.
    pub partition_uuid: [u8; 16],
    pub start_lba: u64,
    pub end_lba: u64,
    /// Bytes of ISO data stored in this chunk.
    pub data_size: u64,
    pub index: u8,
    pub written: bool,
}

impl ChunkInfo {
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

    pub const fn is_valid(&self) -> bool {
        self.start_lba != 0 && self.end_lba > self.start_lba
    }

    /// Partition size in bytes (512-byte sectors).
    pub const fn partition_size(&self) -> u64 {
        (self.end_lba - self.start_lba + 1) * 512
    }

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

/// Chunks for a single ISO; `count` valid entries in a fixed array.
#[derive(Debug, Clone)]
pub struct ChunkSet {
    pub chunks: [ChunkInfo; MAX_CHUNKS],
    pub count: usize,
    pub total_size: u64,
    pub bytes_written: u64,
}

impl ChunkSet {
    pub const fn new() -> Self {
        Self {
            chunks: [ChunkInfo::empty(); MAX_CHUNKS],
            count: 0,
            total_size: 0,
            bytes_written: 0,
        }
    }

    /// Returns the chunk index, or None if the set is full.
    pub fn add_chunk(&mut self, info: ChunkInfo) -> Option<usize> {
        if self.count >= MAX_CHUNKS {
            return None;
        }
        let idx = self.count;
        self.chunks[idx] = info;
        self.count += 1;
        Some(idx)
    }

    pub fn get(&self, index: usize) -> Option<&ChunkInfo> {
        if index < self.count {
            Some(&self.chunks[index])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut ChunkInfo> {
        if index < self.count {
            Some(&mut self.chunks[index])
        } else {
            None
        }
    }

    /// Returns (chunk index, offset within chunk) for a byte offset.
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

    pub fn total_capacity(&self) -> u64 {
        let mut total = 0u64;
        for i in 0..self.count {
            total += self.chunks[i].partition_size();
        }
        total
    }

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

    pub fn progress_percent(&self) -> u8 {
        if self.total_size == 0 {
            return 0;
        }
        ((self.bytes_written * 100) / self.total_size) as u8
    }

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
