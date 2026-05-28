//! Streams a chunked ISO as a single contiguous file, hiding chunk boundaries.

use super::chunk::{ChunkInfo, ChunkSet, MAX_CHUNKS};
use super::error::IsoError;
use super::manifest::IsoManifest;
use super::DEFAULT_CHUNK_SIZE;

pub struct ChunkReader {
    chunks: ChunkSet,
    /// Sequential read position.
    position: u64,
    total_size: u64,
    /// Chunk size used at write time.
    chunk_size: u64,
    current_chunk_cache: usize,
}

impl ChunkReader {
    pub fn from_manifest(manifest: &IsoManifest) -> Result<Self, IsoError> {
        if manifest.chunks.count == 0 {
            return Err(IsoError::ManifestNotFound);
        }

        Ok(Self {
            chunks: manifest.chunks.clone(),
            position: 0,
            total_size: manifest.total_size,
            chunk_size: DEFAULT_CHUNK_SIZE,
            current_chunk_cache: 0,
        })
    }

    pub fn from_chunks(chunks: ChunkSet, chunk_size: u64) -> Self {
        let total_size = chunks.total_size;
        Self {
            chunks,
            position: 0,
            total_size,
            chunk_size,
            current_chunk_cache: 0,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    pub fn num_chunks(&self) -> usize {
        self.chunks.count
    }

    pub fn seek(&mut self, position: u64) -> Result<(), IsoError> {
        if position > self.total_size {
            return Err(IsoError::ReadOverflow);
        }
        self.position = position;
        self.current_chunk_cache = self.chunk_index_for_offset(position).unwrap_or(0);
        Ok(())
    }

    pub fn remaining(&self) -> u64 {
        self.total_size.saturating_sub(self.position)
    }

    pub fn is_eof(&self) -> bool {
        self.position >= self.total_size
    }

    fn chunk_index_for_offset(&self, offset: u64) -> Option<usize> {
        // Fast path: cached chunk.
        if self.current_chunk_cache < self.chunks.count {
            let chunk = &self.chunks.chunks[self.current_chunk_cache];
            let chunk_start = self.chunk_start_offset(self.current_chunk_cache);
            let chunk_end = chunk_start + chunk.data_size;
            if offset >= chunk_start && offset < chunk_end {
                return Some(self.current_chunk_cache);
            }
        }

        let mut cumulative = 0u64;
        for i in 0..self.chunks.count {
            let chunk_size = self.chunks.chunks[i].data_size;
            if offset < cumulative + chunk_size {
                return Some(i);
            }
            cumulative += chunk_size;
        }
        None
    }

    fn chunk_start_offset(&self, chunk_index: usize) -> u64 {
        let mut offset = 0u64;
        for i in 0..chunk_index {
            offset += self.chunks.chunks[i].data_size;
        }
        offset
    }

    /// Reads up to `buffer.len()`; may stop short at EOF or chunk boundary.
    /// `read_sector_fn(partition_start_lba, sector_offset, buf)`.
    pub fn read_at<F>(
        &self,
        offset: u64,
        buffer: &mut [u8],
        mut read_sector_fn: F,
    ) -> Result<usize, IsoError>
    where
        F: FnMut(u64, u64, &mut [u8]) -> Result<usize, IsoError>,
    {
        if offset >= self.total_size {
            return Ok(0);
        }

        let chunk_index = self
            .chunk_index_for_offset(offset)
            .ok_or(IsoError::ReadOverflow)?;
        let chunk = &self.chunks.chunks[chunk_index];

        let chunk_start = self.chunk_start_offset(chunk_index);
        let offset_in_chunk = offset - chunk_start;

        let available_in_chunk = chunk.data_size - offset_in_chunk;
        let available_total = self.total_size - offset;
        let read_size = (buffer.len() as u64)
            .min(available_in_chunk)
            .min(available_total) as usize;

        if read_size == 0 {
            return Ok(0);
        }

        // Matches writer layout: data starts at sector 8192.
        const DATA_START_SECTOR: u64 = 8192;
        let sector_offset = DATA_START_SECTOR + (offset_in_chunk / 512);

        let bytes_read = read_sector_fn(chunk.start_lba, sector_offset, &mut buffer[..read_size])?;

        Ok(bytes_read)
    }

    pub fn read_next<F>(&mut self, buffer: &mut [u8], read_sector_fn: F) -> Result<usize, IsoError>
    where
        F: FnMut(u64, u64, &mut [u8]) -> Result<usize, IsoError>,
    {
        let bytes_read = self.read_at(self.position, buffer, read_sector_fn)?;
        self.position += bytes_read as u64;

        if bytes_read > 0 {
            self.current_chunk_cache = self.chunk_index_for_offset(self.position).unwrap_or(0);
        }

        Ok(bytes_read)
    }

    /// Reads across chunk boundaries transparently.
    pub fn read_range<F>(
        &mut self,
        offset: u64,
        buffer: &mut [u8],
        mut read_sector_fn: F,
    ) -> Result<usize, IsoError>
    where
        F: FnMut(u64, u64, &mut [u8]) -> Result<usize, IsoError>,
    {
        let mut total_read = 0usize;
        let mut current_offset = offset;
        let mut remaining_buffer = buffer;

        while !remaining_buffer.is_empty() && current_offset < self.total_size {
            let bytes_read = self.read_at(current_offset, remaining_buffer, &mut read_sector_fn)?;

            if bytes_read == 0 {
                break;
            }

            total_read += bytes_read;
            current_offset += bytes_read as u64;
            remaining_buffer = &mut remaining_buffer[bytes_read..];
        }

        Ok(total_read)
    }

    pub fn get_chunk(&self, index: usize) -> Option<&ChunkInfo> {
        self.chunks.get(index)
    }

    pub fn chunks(&self) -> impl Iterator<Item = &ChunkInfo> {
        self.chunks.iter()
    }
}

/// Lightweight chunk layout passed to the boot/kernel loader.
#[derive(Clone)]
pub struct IsoReadContext {
    /// Per-chunk partition LBAs (start, end).
    pub chunk_lbas: [(u64, u64); MAX_CHUNKS],
    /// Per-chunk data size.
    pub chunk_sizes: [u64; MAX_CHUNKS],
    pub num_chunks: usize,
    pub total_size: u64,
}

impl IsoReadContext {
    pub fn from_manifest(manifest: &IsoManifest) -> Self {
        let mut chunk_lbas = [(0u64, 0u64); MAX_CHUNKS];
        let mut chunk_sizes = [0u64; MAX_CHUNKS];

        for i in 0..manifest.chunks.count {
            let chunk = &manifest.chunks.chunks[i];
            chunk_lbas[i] = (chunk.start_lba, chunk.end_lba);
            chunk_sizes[i] = chunk.data_size;
        }

        Self {
            chunk_lbas,
            chunk_sizes,
            num_chunks: manifest.chunks.count,
            total_size: manifest.total_size,
        }
    }

    pub fn from_reader(reader: &ChunkReader) -> Self {
        let mut chunk_lbas = [(0u64, 0u64); MAX_CHUNKS];
        let mut chunk_sizes = [0u64; MAX_CHUNKS];

        for i in 0..reader.chunks.count {
            let chunk = &reader.chunks.chunks[i];
            chunk_lbas[i] = (chunk.start_lba, chunk.end_lba);
            chunk_sizes[i] = chunk.data_size;
        }

        Self {
            chunk_lbas,
            chunk_sizes,
            num_chunks: reader.chunks.count,
            total_size: reader.total_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_chunks() -> ChunkSet {
        let mut chunks = ChunkSet::new();
        chunks.total_size = 3_000_000_000;

        let mut c0 = ChunkInfo::new([1u8; 16], 100, 9000000, 0);
        c0.data_size = 1_000_000_000;
        c0.written = true;
        chunks.add_chunk(c0);

        let mut c1 = ChunkInfo::new([2u8; 16], 9000001, 18000000, 1);
        c1.data_size = 1_000_000_000;
        c1.written = true;
        chunks.add_chunk(c1);

        let mut c2 = ChunkInfo::new([3u8; 16], 18000001, 27000000, 2);
        c2.data_size = 1_000_000_000;
        c2.written = true;
        chunks.add_chunk(c2);

        chunks
    }

    #[test]
    fn test_reader_seek() {
        let chunks = make_test_chunks();
        let mut reader = ChunkReader::from_chunks(chunks, 1_000_000_000);

        assert_eq!(reader.position(), 0);
        reader.seek(500_000_000).unwrap();
        assert_eq!(reader.position(), 500_000_000);

        reader.seek(1_000_000_000).unwrap();
        assert_eq!(reader.position(), 1_000_000_000);

        assert!(reader.seek(4_000_000_000).is_err());
    }

    #[test]
    fn test_chunk_index_lookup() {
        let chunks = make_test_chunks();
        let reader = ChunkReader::from_chunks(chunks, 1_000_000_000);

        assert_eq!(reader.chunk_index_for_offset(0), Some(0));
        assert_eq!(reader.chunk_index_for_offset(500_000_000), Some(0));
        assert_eq!(reader.chunk_index_for_offset(999_999_999), Some(0));
        assert_eq!(reader.chunk_index_for_offset(1_000_000_000), Some(1));
        assert_eq!(reader.chunk_index_for_offset(2_500_000_000), Some(2));
        assert_eq!(reader.chunk_index_for_offset(3_000_000_000), None);
    }
}
