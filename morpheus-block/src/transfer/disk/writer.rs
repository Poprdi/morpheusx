//! Post-EBS streaming ISO writer; splits across FAT32 chunk partitions.

use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

use super::fat32::{Fat32Formatter, Fat32Info};
use super::gpt::GptOps;
use super::types::{
    guid, ChunkPartition, ChunkSet, DiskError, DiskResult, PartitionInfo, DEFAULT_CHUNK_SIZE,
    MAX_CHUNK_PARTITIONS, SECTOR_SIZE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterState {
    Uninitialized,
    Ready,
    Writing,
    Complete,
    Failed,
}

pub type ProgressFn = fn(bytes_written: u64, total_bytes: u64, chunk: usize, total_chunks: usize);

/// Creates partitions, formats FAT32, streams data.
#[allow(dead_code)]
pub struct IsoWriter {
    state: WriterState,
    chunks: ChunkSet,
    fat32_info: [Option<Fat32Info>; MAX_CHUNK_PARTITIONS],
    current_chunk: usize,
    chunk_bytes: u64,
    total_bytes: u64,
    total_size: u64,
    chunk_size: u64,
    iso_name: [u8; 64],
    name_len: usize,
    progress_fn: Option<ProgressFn>,
}

impl IsoWriter {
    pub fn new(iso_name: &str, total_size: u64) -> Self {
        let mut name = [0u8; 64];
        let len = iso_name.len().min(63);
        name[..len].copy_from_slice(&iso_name.as_bytes()[..len]);

        Self {
            state: WriterState::Uninitialized,
            chunks: ChunkSet::new(),
            fat32_info: [None; MAX_CHUNK_PARTITIONS],
            current_chunk: 0,
            chunk_bytes: 0,
            total_bytes: 0,
            total_size,
            chunk_size: DEFAULT_CHUNK_SIZE,
            iso_name: name,
            name_len: len,
            progress_fn: None,
        }
    }

    pub fn set_progress(&mut self, f: ProgressFn) {
        self.progress_fn = Some(f);
    }

    pub fn state(&self) -> WriterState {
        self.state
    }

    pub fn bytes_written(&self) -> u64 {
        self.total_bytes
    }

    pub fn chunks(&self) -> &ChunkSet {
        &self.chunks
    }

    /// Create partitions and format FAT32. Call before any write.
    pub fn initialize<B: BlockIo>(&mut self, block_io: &mut B) -> DiskResult<()> {
        if self.state != WriterState::Uninitialized {
            return Err(DiskError::InvalidParameter);
        }

        let num_chunks = self.total_size.div_ceil(self.chunk_size) as usize;
        if num_chunks > MAX_CHUNK_PARTITIONS {
            return Err(DiskError::InvalidSize);
        }

        self.chunks.total_size = self.total_size;

        for i in 0..num_chunks {
            let mut name_buf = [0u8; 16];
            let chunk_name = self.chunk_name_str(i, &mut name_buf);

            let (start, end) = GptOps::find_free_space(block_io)?;

            // chunk_size sectors + FAT32 overhead.
            let sectors_needed = (self.chunk_size / SECTOR_SIZE as u64) + 8192;
            let available = end - start + 1;

            if available < sectors_needed {
                if i == 0 {
                    return Err(DiskError::NoFreeSpace);
                }
                break;
            }

            let part_end = start + sectors_needed - 1;

            let slot =
                GptOps::create_partition(block_io, start, part_end, guid::BASIC_DATA, chunk_name)?;

            let fat32_info = Fat32Formatter::format(block_io, start, sectors_needed, chunk_name)?;

            let mut part_info = PartitionInfo::new(slot, start, part_end, guid::BASIC_DATA);
            part_info.set_name(chunk_name);

            let chunk = ChunkPartition::new(part_info, i as u8);
            self.chunks.add(chunk)?;
            self.fat32_info[i] = Some(fat32_info);
        }

        self.state = WriterState::Ready;
        Ok(())
    }

    /// Use partitions created earlier (pre-EBS path). `partitions`: (start_lba, end_lba) pairs.
    pub fn initialize_with_partitions(
        &mut self,
        partitions: &[(u64, u64)],
        fat32_infos: &[Fat32Info],
    ) -> DiskResult<()> {
        if self.state != WriterState::Uninitialized {
            return Err(DiskError::InvalidParameter);
        }

        if partitions.len() != fat32_infos.len() {
            return Err(DiskError::InvalidParameter);
        }

        self.chunks.total_size = self.total_size;

        for (i, (&(start, end), &info)) in partitions.iter().zip(fat32_infos.iter()).enumerate() {
            let mut part_info = PartitionInfo::new(i as u8, start, end, guid::BASIC_DATA);
            let mut name_buf = [0u8; 16];
            let name = self.chunk_name_str(i, &mut name_buf);
            part_info.set_name(name);

            let chunk = ChunkPartition::new(part_info, i as u8);
            self.chunks.add(chunk)?;
            self.fat32_info[i] = Some(info);
        }

        self.state = WriterState::Ready;
        Ok(())
    }

    /// Auto-advances across chunk boundaries.
    pub fn write<B: BlockIo>(&mut self, block_io: &mut B, data: &[u8]) -> DiskResult<usize> {
        if self.state == WriterState::Complete {
            return Err(DiskError::WriteOverflow);
        }
        if self.state != WriterState::Ready && self.state != WriterState::Writing {
            return Err(DiskError::InvalidParameter);
        }

        self.state = WriterState::Writing;

        let mut written = 0usize;
        let mut remaining = data;

        while !remaining.is_empty() && self.total_bytes < self.total_size {
            if self.chunk_bytes >= self.chunk_size {
                self.current_chunk += 1;
                self.chunk_bytes = 0;

                if self.current_chunk >= self.chunks.count {
                    self.state = WriterState::Failed;
                    return Err(DiskError::WriteOverflow);
                }
            }

            let chunk_space = self.chunk_size - self.chunk_bytes;
            let total_space = self.total_size - self.total_bytes;
            let write_size = (remaining.len() as u64).min(chunk_space).min(total_space) as usize;

            if write_size == 0 {
                break;
            }

            let _chunk = &self.chunks.chunks[self.current_chunk];
            let fat32 = self.fat32_info[self.current_chunk].ok_or(DiskError::InvalidParameter)?;

            let sector_offset = self.chunk_bytes / SECTOR_SIZE as u64;
            let write_lba = fat32.data_start_lba + sector_offset;

            self.write_sectors(block_io, write_lba, &remaining[..write_size])?;

            self.chunk_bytes += write_size as u64;
            self.total_bytes += write_size as u64;
            written += write_size;
            remaining = &remaining[write_size..];

            if let Some(c) = self.chunks.get_mut(self.current_chunk) {
                c.bytes_written = self.chunk_bytes;
            }

            if let Some(f) = self.progress_fn {
                f(
                    self.total_bytes,
                    self.total_size,
                    self.current_chunk,
                    self.chunks.count,
                );
            }
        }

        if self.total_bytes >= self.total_size {
            self.state = WriterState::Complete;
            if let Some(c) = self.chunks.get_mut(self.current_chunk) {
                c.complete = true;
            }
        }

        self.chunks.bytes_written = self.total_bytes;
        Ok(written)
    }

    fn write_sectors<B: BlockIo>(
        &self,
        block_io: &mut B,
        start_lba: u64,
        data: &[u8],
    ) -> DiskResult<()> {
        // Partial sector at head needs RMW.
        let offset_in_sector = (self.chunk_bytes % SECTOR_SIZE as u64) as usize;
        let mut current_lba = start_lba;
        let mut remaining = data;

        if offset_in_sector != 0 || data.len() < SECTOR_SIZE {
            let mut sector_buf = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(Lba(current_lba), &mut sector_buf)
                .map_err(|_| DiskError::IoError)?;

            let copy_len = (SECTOR_SIZE - offset_in_sector).min(remaining.len());
            sector_buf[offset_in_sector..offset_in_sector + copy_len]
                .copy_from_slice(&remaining[..copy_len]);

            block_io
                .write_blocks(Lba(current_lba), &sector_buf)
                .map_err(|_| DiskError::IoError)?;

            current_lba += 1;
            remaining = &remaining[copy_len..];
        }

        let full_sectors = remaining.len() / SECTOR_SIZE;
        for i in 0..full_sectors {
            let sector_data = &remaining[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .write_blocks(Lba(current_lba + i as u64), sector_data)
                .map_err(|_| DiskError::IoError)?;
        }

        // Trailing partial sector: RMW.
        let trailing_bytes = remaining.len() % SECTOR_SIZE;
        if trailing_bytes > 0 {
            let trailing_lba = current_lba + full_sectors as u64;
            let trailing_data = &remaining[full_sectors * SECTOR_SIZE..];

            let mut sector_buf = [0u8; SECTOR_SIZE];
            let _ = block_io.read_blocks(Lba(trailing_lba), &mut sector_buf);

            sector_buf[..trailing_bytes].copy_from_slice(trailing_data);
            block_io
                .write_blocks(Lba(trailing_lba), &sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        Ok(())
    }

    pub fn finalize<B: BlockIo>(&mut self, block_io: &mut B) -> DiskResult<()> {
        block_io.flush().map_err(|_| DiskError::IoError)?;

        if self.state == WriterState::Writing {
            self.state = WriterState::Complete;
        }

        Ok(())
    }

    /// "ISO_CHK_NN".
    fn chunk_name_str<'a>(&self, index: usize, buf: &'a mut [u8; 16]) -> &'a str {
        buf[0..8].copy_from_slice(b"ISO_CHK_");
        buf[8] = b'0' + (index / 10) as u8;
        buf[9] = b'0' + (index % 10) as u8;
        buf[10] = 0;
        core::str::from_utf8(&buf[..10]).unwrap_or("ISO_CHK_00")
    }
}
