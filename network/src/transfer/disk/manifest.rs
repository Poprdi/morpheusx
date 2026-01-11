//! Manifest writer for post-EBS.
//!
//! Binary manifest format for tracking ISO chunks.
//! Written to ESP for bootloader to locate ISO data.

#[cfg(feature = "fat32_manifest")]
extern crate alloc;

use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

use super::types::{
    ChunkSet, DiskError, DiskResult, MAX_CHUNK_PARTITIONS, MAX_ISO_NAME_LEN, SECTOR_SIZE,
};

/// Manifest magic: "MXISO\x01\x00\x00" (v1 - compatible with morpheus_core)
pub const MANIFEST_MAGIC: [u8; 8] = [b'M', b'X', b'I', b'S', b'O', 0x01, 0x00, 0x00];

/// Manifest header size
pub const MANIFEST_HEADER_SIZE: usize = 128;

/// Chunk entry size  
pub const CHUNK_ENTRY_SIZE: usize = 48;

/// Maximum manifest size (header + 16 chunks)
pub const MAX_MANIFEST_SIZE: usize =
    MANIFEST_HEADER_SIZE + (MAX_CHUNK_PARTITIONS * CHUNK_ENTRY_SIZE);

/// Manifest flags
pub mod flags {
    pub const COMPLETE: u8 = 0x01;
    pub const VERIFIED: u8 = 0x02;
}

/// Binary manifest for ISO chunks
///
/// Format:
/// ```text
/// Offset  Size  Field
/// 0x00    8     Magic "MXISO\x02\x00\x00"
/// 0x08    64    ISO name (null-terminated)
/// 0x48    8     Total size (u64 LE)
/// 0x50    32    SHA256 hash (or zeros)
/// 0x70    1     Number of chunks
/// 0x71    1     Flags
/// 0x72    2     Reserved
/// 0x74    4     Header CRC32
/// 0x78    8     Reserved
/// 0x80    N*48  Chunk entries
/// ```
pub struct ManifestWriter {
    /// ISO name
    name: [u8; MAX_ISO_NAME_LEN],
    /// Name length
    name_len: usize,
    /// Total ISO size
    total_size: u64,
    /// SHA256 hash
    sha256: [u8; 32],
    /// Flags
    flags: u8,
}

impl ManifestWriter {
    /// Create new manifest writer
    pub fn new(iso_name: &str, total_size: u64) -> Self {
        let mut name = [0u8; MAX_ISO_NAME_LEN];
        let len = iso_name.as_bytes().len().min(MAX_ISO_NAME_LEN - 1);
        name[..len].copy_from_slice(&iso_name.as_bytes()[..len]);

        Self {
            name,
            name_len: len,
            total_size,
            sha256: [0u8; 32],
            flags: 0,
        }
    }

    /// Set SHA256 hash
    pub fn set_hash(&mut self, hash: &[u8; 32]) {
        self.sha256.copy_from_slice(hash);
    }

    /// Mark as complete
    pub fn set_complete(&mut self, complete: bool) {
        if complete {
            self.flags |= flags::COMPLETE;
        } else {
            self.flags &= !flags::COMPLETE;
        }
    }

    /// Mark as verified
    pub fn set_verified(&mut self, verified: bool) {
        if verified {
            self.flags |= flags::VERIFIED;
        } else {
            self.flags &= !flags::VERIFIED;
        }
    }

    /// Serialize manifest to buffer
    ///
    /// Returns number of bytes written.
    pub fn serialize(&self, chunks: &ChunkSet, buffer: &mut [u8]) -> DiskResult<usize> {
        let needed = MANIFEST_HEADER_SIZE + (chunks.count * CHUNK_ENTRY_SIZE);
        if buffer.len() < needed {
            return Err(DiskError::BufferTooSmall);
        }

        buffer[..needed].fill(0);

        // Magic
        buffer[0..8].copy_from_slice(&MANIFEST_MAGIC);

        // Name
        buffer[8..8 + self.name_len].copy_from_slice(&self.name[..self.name_len]);

        // Total size
        buffer[0x48..0x50].copy_from_slice(&self.total_size.to_le_bytes());

        // SHA256
        buffer[0x50..0x70].copy_from_slice(&self.sha256);

        // Number of chunks
        buffer[0x70] = chunks.count as u8;

        // Flags
        buffer[0x71] = self.flags;

        // Chunk entries
        for i in 0..chunks.count {
            let chunk = &chunks.chunks[i];
            let offset = MANIFEST_HEADER_SIZE + (i * CHUNK_ENTRY_SIZE);

            // Partition UUID (use type GUID for now)
            buffer[offset..offset + 16].copy_from_slice(&chunk.info.type_guid);

            // Start LBA
            buffer[offset + 16..offset + 24].copy_from_slice(&chunk.info.start_lba.to_le_bytes());

            // End LBA
            buffer[offset + 24..offset + 32].copy_from_slice(&chunk.info.end_lba.to_le_bytes());

            // Data size
            buffer[offset + 32..offset + 40].copy_from_slice(&chunk.bytes_written.to_le_bytes());

            // Chunk index
            buffer[offset + 40] = chunk.chunk_index;

            // Flags (written bit)
            buffer[offset + 41] = if chunk.complete { 0x01 } else { 0x00 };
        }

        // Calculate and write header CRC32
        let header_crc = crc32(&buffer[0..0x74]);
        buffer[0x74..0x78].copy_from_slice(&header_crc.to_le_bytes());

        Ok(needed)
    }

    /// Write manifest to ESP partition
    ///
    /// Writes to `/morpheus/isos/<name>.manifest` conceptually,
    /// but since we can't do FAT32 file ops without alloc, we write
    /// to a fixed location within the ESP.
    ///
    /// # Arguments
    /// * `block_io` - Block I/O device
    /// * `esp_start_lba` - Start LBA of ESP partition
    /// * `manifest_offset` - Sector offset within ESP for manifest storage
    /// * `chunks` - Chunk information to write
    pub fn write_to_esp<B: BlockIo>(
        &self,
        block_io: &mut B,
        esp_start_lba: u64,
        manifest_offset: u64, // Sector offset within ESP
        chunks: &ChunkSet,
    ) -> DiskResult<()> {
        // Serialize to buffer (using 2 sectors = 1024 bytes max)
        let mut buffer = [0u8; SECTOR_SIZE * 2];
        let len = self.serialize(chunks, &mut buffer)?;

        // Write manifest sectors
        let manifest_lba = esp_start_lba + manifest_offset;
        let sectors_needed = (len + SECTOR_SIZE - 1) / SECTOR_SIZE;

        for i in 0..sectors_needed {
            let sector_data = &buffer[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .write_blocks(Lba(manifest_lba + i as u64), sector_data)
                .map_err(|_| DiskError::IoError)?;
        }

        block_io.flush().map_err(|_| DiskError::IoError)?;

        Ok(())
    }

    /// Write manifest as FAT32 file to ESP
    ///
    /// Writes to `/morpheus/isos/<short_name>.mfst` on the ESP.
    /// Uses 8.3 compatible filenames for FAT32 compatibility.
    /// This is the preferred method as it integrates with the bootloader scanner.
    ///
    /// # Requirements
    /// - Heap allocator must be initialized (crate::alloc_heap::init_heap())
    /// - ESP must be a valid FAT32 partition
    ///
    /// # Arguments
    /// * `block_io` - Block I/O device
    /// * `esp_start_lba` - Start LBA of ESP partition
    /// * `chunks` - Chunk information to write
    #[cfg(feature = "fat32_manifest")]
    pub fn write_to_esp_fat32<B: BlockIo>(
        &self,
        block_io: &mut B,
        esp_start_lba: u64,
        chunks: &ChunkSet,
    ) -> DiskResult<()> {
        use alloc::format;
        use alloc::string::String;

        // Serialize manifest to buffer
        let mut buffer = [0u8; MAX_MANIFEST_SIZE];
        let len = self.serialize(chunks, &mut buffer)?;

        // Build 8.3 compatible filename from ISO name
        // e.g., "tails-amd64-7.3.1.iso" -> "TAILS731.MFS"
        let name_str = core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("unknown");
        let short_name = make_8_3_filename(name_str);
        let path = format!("/morpheus/isos/{}", short_name);

        // Ensure directory exists
        let _ = morpheus_core::fs::create_directory(block_io, esp_start_lba, "/morpheus");
        let _ = morpheus_core::fs::create_directory(block_io, esp_start_lba, "/morpheus/isos");

        // Write manifest file
        morpheus_core::fs::write_file(block_io, esp_start_lba, &path, &buffer[..len])
            .map_err(|_| DiskError::ManifestError)?;

        Ok(())
    }
}

/// Generate an 8.3 compatible filename for FAT32
/// Takes ISO name like "tails-amd64-7.3.1.iso" and returns "TAILS731.MFS"
fn make_8_3_filename(name: &str) -> alloc::string::String {
    use alloc::string::String;

    // Extract meaningful characters, uppercase, alphanumeric only
    let clean: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_uppercase();

    // Ensure we have at least something
    let base = if clean.is_empty() { "MANIFEST" } else { &clean };

    // Use .MFS extension (short for manifest)
    alloc::format!("{}.MFS", base)
}

/// Manifest reader for loading existing manifests
pub struct ManifestReader;

impl ManifestReader {
    /// Read manifest from buffer
    ///
    /// Returns (name, total_size, chunks) if valid.
    pub fn parse(buffer: &[u8]) -> DiskResult<(IsoManifestInfo, ChunkSet)> {
        if buffer.len() < MANIFEST_HEADER_SIZE {
            return Err(DiskError::BufferTooSmall);
        }

        // Check magic
        if &buffer[0..8] != &MANIFEST_MAGIC {
            return Err(DiskError::ManifestError);
        }

        // Verify CRC32
        let stored_crc = u32::from_le_bytes(buffer[0x74..0x78].try_into().unwrap());
        let mut check_buf = [0u8; 0x74];
        check_buf.copy_from_slice(&buffer[0..0x74]);
        let calc_crc = crc32(&check_buf);
        if stored_crc != calc_crc {
            return Err(DiskError::ManifestError);
        }

        // Parse header
        let mut name = [0u8; MAX_ISO_NAME_LEN];
        name.copy_from_slice(&buffer[8..8 + MAX_ISO_NAME_LEN]);

        let total_size = u64::from_le_bytes(buffer[0x48..0x50].try_into().unwrap());

        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&buffer[0x50..0x70]);

        let num_chunks = buffer[0x70] as usize;
        let flags = buffer[0x71];

        if num_chunks > MAX_CHUNK_PARTITIONS {
            return Err(DiskError::ManifestError);
        }

        // Parse chunks
        let mut chunks = ChunkSet::new();
        chunks.total_size = total_size;

        for i in 0..num_chunks {
            let offset = MANIFEST_HEADER_SIZE + (i * CHUNK_ENTRY_SIZE);
            if offset + CHUNK_ENTRY_SIZE > buffer.len() {
                return Err(DiskError::BufferTooSmall);
            }

            let mut type_guid = [0u8; 16];
            type_guid.copy_from_slice(&buffer[offset..offset + 16]);

            let start_lba =
                u64::from_le_bytes(buffer[offset + 16..offset + 24].try_into().unwrap());
            let end_lba = u64::from_le_bytes(buffer[offset + 24..offset + 32].try_into().unwrap());
            let data_size =
                u64::from_le_bytes(buffer[offset + 32..offset + 40].try_into().unwrap());
            let chunk_index = buffer[offset + 40];
            let chunk_flags = buffer[offset + 41];

            let part_info =
                super::types::PartitionInfo::new(i as u8, start_lba, end_lba, type_guid);
            let mut chunk = super::types::ChunkPartition::new(part_info, chunk_index);
            chunk.bytes_written = data_size;
            chunk.complete = chunk_flags & 0x01 != 0;

            chunks.add(chunk)?;
            chunks.bytes_written += data_size;
        }

        let info = IsoManifestInfo {
            name,
            total_size,
            sha256,
            flags,
        };

        Ok((info, chunks))
    }

    /// Read manifest from ESP
    pub fn read_from_esp<B: BlockIo>(
        block_io: &mut B,
        esp_start_lba: u64,
        manifest_offset: u64,
    ) -> DiskResult<(IsoManifestInfo, ChunkSet)> {
        // Read 2 sectors
        let mut buffer = [0u8; SECTOR_SIZE * 2];
        block_io
            .read_blocks(
                Lba(esp_start_lba + manifest_offset),
                &mut buffer[0..SECTOR_SIZE],
            )
            .map_err(|_| DiskError::IoError)?;
        block_io
            .read_blocks(
                Lba(esp_start_lba + manifest_offset + 1),
                &mut buffer[SECTOR_SIZE..],
            )
            .map_err(|_| DiskError::IoError)?;

        Self::parse(&buffer)
    }
}

/// Parsed manifest information
#[derive(Debug, Clone)]
pub struct IsoManifestInfo {
    /// ISO name
    pub name: [u8; MAX_ISO_NAME_LEN],
    /// Total size
    pub total_size: u64,
    /// SHA256 hash
    pub sha256: [u8; 32],
    /// Flags
    pub flags: u8,
}

impl IsoManifestInfo {
    /// Get name as str
    pub fn name_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    /// Check if complete
    pub fn is_complete(&self) -> bool {
        self.flags & flags::COMPLETE != 0
    }

    /// Check if verified
    pub fn is_verified(&self) -> bool {
        self.flags & flags::VERIFIED != 0
    }
}

/// CRC32 (IEEE 802.3)
fn crc32(data: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0xEDB88320;
    let mut crc = 0xFFFF_FFFFu32;

    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLYNOMIAL;
            } else {
                crc >>= 1;
            }
        }
    }

    !crc
}
