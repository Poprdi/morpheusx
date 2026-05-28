//! Binary manifest tracking ISO chunks, stored on ESP at `/.iso/<name>.manifest`.
//!
//! Binary format v1:
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//! 0x00    8     Magic number "MXISO\x01\x00\x00"
//! 0x08    64    ISO filename (null-terminated, padded)
//! 0x48    8     Total ISO size (little-endian u64)
//! 0x50    32    SHA256 hash (or zeros if not verified)
//! 0x70    1     Number of chunks
//! 0x71    1     Flags (bit 0 = complete, bit 1 = verified)
//! 0x72    2     Reserved
//! 0x74    4     CRC32 of header (offset 0x00-0x73)
//! 0x78    8     Reserved (align to 128 bytes)
//! 0x80    N*48  Chunk entries (48 bytes each)
//!
//! Chunk Entry (48 bytes):
//! 0x00    16    Partition UUID
//! 0x10    8     Start LBA
//! 0x18    8     End LBA  
//! 0x20    8     Data size in this chunk
//! 0x28    1     Chunk index
//! 0x29    1     Flags (bit 0 = written)
//! 0x2A    6     Reserved
//! ```
//!
//! Total header size: 128 + (num_chunks * 48) bytes

use super::chunk::{ChunkInfo, ChunkSet, MAX_CHUNKS};
use super::error::IsoError;

pub const MANIFEST_MAGIC: [u8; 8] = [b'M', b'X', b'I', b'S', b'O', 0x01, 0x00, 0x00];

pub const MANIFEST_HEADER_SIZE: usize = 128;

pub const CHUNK_ENTRY_SIZE: usize = 48;

pub const MAX_MANIFEST_SIZE: usize = MANIFEST_HEADER_SIZE + (MAX_CHUNKS * CHUNK_ENTRY_SIZE);

pub const MAX_ISO_NAME_LEN: usize = 64;

pub mod flags {
    pub const COMPLETE: u8 = 0x01;
    pub const VERIFIED: u8 = 0x02;
}

#[derive(Clone)]
pub struct IsoManifest {
    /// Null-terminated filename.
    pub name: [u8; MAX_ISO_NAME_LEN],
    /// Length excluding null terminator.
    pub name_len: usize,
    pub total_size: u64,
    /// SHA256, zeros if unset.
    pub sha256: [u8; 32],
    pub chunks: ChunkSet,
    pub flags: u8,
}

impl IsoManifest {
    pub fn new(name: &str, total_size: u64) -> Self {
        let mut manifest = Self {
            name: [0u8; MAX_ISO_NAME_LEN],
            name_len: 0,
            total_size,
            sha256: [0u8; 32],
            chunks: ChunkSet::new(),
            flags: 0,
        };
        manifest.set_name(name);
        manifest.chunks.total_size = total_size;
        manifest
    }

    pub fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(MAX_ISO_NAME_LEN - 1);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
        self.name_len = len;
    }

    pub fn name_str(&self) -> &str {
        // SAFETY: set_name only stores valid UTF-8.
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    pub fn set_sha256(&mut self, hash: &[u8; 32]) {
        self.sha256.copy_from_slice(hash);
    }

    pub fn is_complete(&self) -> bool {
        self.flags & flags::COMPLETE != 0
    }

    pub fn mark_complete(&mut self) {
        self.flags |= flags::COMPLETE;
    }

    pub fn is_verified(&self) -> bool {
        self.flags & flags::VERIFIED != 0
    }

    pub fn mark_verified(&mut self) {
        self.flags |= flags::VERIFIED;
    }

    pub fn add_chunk(
        &mut self,
        partition_uuid: [u8; 16],
        start_lba: u64,
        end_lba: u64,
    ) -> Result<usize, IsoError> {
        let index = self.chunks.count;
        if index >= MAX_CHUNKS {
            return Err(IsoError::IsoTooLarge);
        }

        let info = ChunkInfo::new(partition_uuid, start_lba, end_lba, index as u8);
        self.chunks.add_chunk(info).ok_or(IsoError::IsoTooLarge)
    }

    pub fn serialized_size(&self) -> usize {
        MANIFEST_HEADER_SIZE + (self.chunks.count * CHUNK_ENTRY_SIZE)
    }

    /// Serializes per the v1 binary layout; buffer >= `serialized_size()`.
    pub fn serialize(&self, buffer: &mut [u8]) -> Result<usize, IsoError> {
        let size = self.serialized_size();
        if buffer.len() < size {
            return Err(IsoError::IoError);
        }

        buffer[..size].fill(0);

        buffer[0..8].copy_from_slice(&MANIFEST_MAGIC);
        buffer[8..8 + self.name_len].copy_from_slice(&self.name[..self.name_len]);
        buffer[0x48..0x50].copy_from_slice(&self.total_size.to_le_bytes());
        buffer[0x50..0x70].copy_from_slice(&self.sha256);
        buffer[0x70] = self.chunks.count as u8;
        buffer[0x71] = self.flags;

        // CRC32 over header bytes 0x00-0x73, written last.
        let crc = crc32(&buffer[0..0x74]);
        buffer[0x74..0x78].copy_from_slice(&crc.to_le_bytes());

        for i in 0..self.chunks.count {
            let chunk = &self.chunks.chunks[i];
            let offset = MANIFEST_HEADER_SIZE + (i * CHUNK_ENTRY_SIZE);

            buffer[offset..offset + 16].copy_from_slice(&chunk.partition_uuid);
            buffer[offset + 0x10..offset + 0x18].copy_from_slice(&chunk.start_lba.to_le_bytes());
            buffer[offset + 0x18..offset + 0x20].copy_from_slice(&chunk.end_lba.to_le_bytes());
            buffer[offset + 0x20..offset + 0x28].copy_from_slice(&chunk.data_size.to_le_bytes());
            buffer[offset + 0x28] = chunk.index;
            buffer[offset + 0x29] = if chunk.written { 0x01 } else { 0x00 };
        }

        Ok(size)
    }

    pub fn deserialize(buffer: &[u8]) -> Result<Self, IsoError> {
        if buffer.len() < MANIFEST_HEADER_SIZE {
            return Err(IsoError::InvalidManifest);
        }

        if buffer[0..8] != MANIFEST_MAGIC {
            return Err(IsoError::InvalidManifest);
        }

        let stored_crc =
            u32::from_le_bytes([buffer[0x74], buffer[0x75], buffer[0x76], buffer[0x77]]);
        let computed_crc = crc32(&buffer[0..0x74]);
        if stored_crc != computed_crc {
            return Err(IsoError::DataCorruption);
        }

        let mut name = [0u8; MAX_ISO_NAME_LEN];
        let mut name_len = 0;
        for i in 0..MAX_ISO_NAME_LEN {
            if buffer[8 + i] == 0 {
                break;
            }
            name[i] = buffer[8 + i];
            name_len = i + 1;
        }

        let total_size = u64::from_le_bytes([
            buffer[0x48],
            buffer[0x49],
            buffer[0x4A],
            buffer[0x4B],
            buffer[0x4C],
            buffer[0x4D],
            buffer[0x4E],
            buffer[0x4F],
        ]);

        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&buffer[0x50..0x70]);

        let chunk_count = buffer[0x70] as usize;
        if chunk_count > MAX_CHUNKS {
            return Err(IsoError::InvalidManifest);
        }

        let flags = buffer[0x71];

        let required_size = MANIFEST_HEADER_SIZE + (chunk_count * CHUNK_ENTRY_SIZE);
        if buffer.len() < required_size {
            return Err(IsoError::InvalidManifest);
        }

        let mut chunks = ChunkSet::new();
        chunks.total_size = total_size;

        for i in 0..chunk_count {
            let offset = MANIFEST_HEADER_SIZE + (i * CHUNK_ENTRY_SIZE);

            let mut partition_uuid = [0u8; 16];
            partition_uuid.copy_from_slice(&buffer[offset..offset + 16]);

            let start_lba = u64::from_le_bytes([
                buffer[offset + 0x10],
                buffer[offset + 0x11],
                buffer[offset + 0x12],
                buffer[offset + 0x13],
                buffer[offset + 0x14],
                buffer[offset + 0x15],
                buffer[offset + 0x16],
                buffer[offset + 0x17],
            ]);

            let end_lba = u64::from_le_bytes([
                buffer[offset + 0x18],
                buffer[offset + 0x19],
                buffer[offset + 0x1A],
                buffer[offset + 0x1B],
                buffer[offset + 0x1C],
                buffer[offset + 0x1D],
                buffer[offset + 0x1E],
                buffer[offset + 0x1F],
            ]);

            let data_size = u64::from_le_bytes([
                buffer[offset + 0x20],
                buffer[offset + 0x21],
                buffer[offset + 0x22],
                buffer[offset + 0x23],
                buffer[offset + 0x24],
                buffer[offset + 0x25],
                buffer[offset + 0x26],
                buffer[offset + 0x27],
            ]);

            let index = buffer[offset + 0x28];
            let written = buffer[offset + 0x29] & 0x01 != 0;

            let mut info = ChunkInfo::new(partition_uuid, start_lba, end_lba, index);
            info.data_size = data_size;
            info.written = written;

            chunks.add_chunk(info);
        }

        let mut bytes_written = 0u64;
        for i in 0..chunks.count {
            if chunks.chunks[i].written {
                bytes_written += chunks.chunks[i].data_size;
            }
        }
        chunks.bytes_written = bytes_written;

        Ok(Self {
            name,
            name_len,
            total_size,
            sha256,
            chunks,
            flags,
        })
    }
}

impl Default for IsoManifest {
    fn default() -> Self {
        Self::new("", 0)
    }
}

/// CRC32, IEEE 802.3 polynomial.
fn crc32(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = generate_crc32_table();

    let mut crc = 0xFFFFFFFF;
    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[index];
    }
    !crc
}

const fn generate_crc32_table() -> [u32; 256] {
    const POLYNOMIAL: u32 = 0xEDB88320;
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLYNOMIAL;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_roundtrip() {
        let mut manifest = IsoManifest::new("ubuntu-24.04.iso", 6_000_000_000);
        manifest.add_chunk([1u8; 16], 1000, 9000000).unwrap();
        manifest.add_chunk([2u8; 16], 9000001, 18000000).unwrap();
        manifest.mark_complete();

        let mut buffer = [0u8; MAX_MANIFEST_SIZE];
        let size = manifest.serialize(&mut buffer).unwrap();

        let restored = IsoManifest::deserialize(&buffer[..size]).unwrap();

        assert_eq!(restored.name_str(), "ubuntu-24.04.iso");
        assert_eq!(restored.total_size, 6_000_000_000);
        assert_eq!(restored.chunks.count, 2);
        assert!(restored.is_complete());
    }

    #[test]
    fn test_crc32() {
        let crc = crc32(b"123456789");
        assert_eq!(crc, 0xCBF43926);
    }
}
