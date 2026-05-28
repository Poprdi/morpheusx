//! Shared types for post-EBS disk ops. Alloc-free, fixed-size arrays.

pub const SECTOR_SIZE: usize = 512;

pub const MAX_CHUNK_PARTITIONS: usize = 16;

pub const MAX_ISO_NAME_LEN: usize = 64;

#[allow(dead_code)]
pub const FAT32_MAX_FILE_SIZE: u64 = 0xFFFF_FFFF;

/// Just under 4 GiB to stay safely inside FAT32's per-file cap.
pub const DEFAULT_CHUNK_SIZE: u64 = 0xFFFF_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskError {
    IoError,
    InvalidGpt,
    NoFreeSpace,
    PartitionNotFound,
    InvalidSize,
    FormatError,
    /// Exceeded partition bounds.
    WriteOverflow,
    InvalidParameter,
    NotSupported,
    ManifestError,
    BufferTooSmall,
}

impl core::fmt::Display for DiskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IoError => write!(f, "I/O error"),
            Self::InvalidGpt => write!(f, "Invalid GPT"),
            Self::NoFreeSpace => write!(f, "No free space"),
            Self::PartitionNotFound => write!(f, "Partition not found"),
            Self::InvalidSize => write!(f, "Invalid size"),
            Self::FormatError => write!(f, "FAT32 format error"),
            Self::WriteOverflow => write!(f, "Write overflow"),
            Self::InvalidParameter => write!(f, "Invalid parameter"),
            Self::NotSupported => write!(f, "Not supported"),
            Self::ManifestError => write!(f, "Manifest error"),
            Self::BufferTooSmall => write!(f, "Buffer too small"),
        }
    }
}

pub type DiskResult<T> = Result<T, DiskError>;

/// GPT partition type GUIDs.
pub mod guid {
    #[allow(dead_code)]
    pub const EFI_SYSTEM: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
        0x3B,
    ];

    pub const BASIC_DATA: [u8; 16] = [
        0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99,
        0xC7,
    ];

    #[allow(dead_code)]
    pub const LINUX_FS: [u8; 16] = [
        0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D,
        0xE4,
    ];
}

#[derive(Debug, Clone, Copy)]
pub struct PartitionInfo {
    /// GPT entry index (0..128).
    pub index: u8,
    pub start_lba: u64,
    /// Inclusive.
    pub end_lba: u64,
    pub type_guid: [u8; 16],
    /// ASCII, null-terminated.
    pub name: [u8; 36],
}

impl Default for PartitionInfo {
    fn default() -> Self {
        Self {
            index: 0,
            start_lba: 0,
            end_lba: 0,
            type_guid: [0; 16],
            name: [0; 36],
        }
    }
}

impl PartitionInfo {
    pub fn new(index: u8, start_lba: u64, end_lba: u64, type_guid: [u8; 16]) -> Self {
        Self {
            index,
            start_lba,
            end_lba,
            type_guid,
            name: [0u8; 36],
        }
    }

    pub fn set_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(35);
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }

    pub fn size_sectors(&self) -> u64 {
        self.end_lba.saturating_sub(self.start_lba) + 1
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_sectors() * SECTOR_SIZE as u64
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ChunkPartition {
    pub info: PartitionInfo,
    pub chunk_index: u8,
    pub bytes_written: u64,
    pub complete: bool,
}

impl ChunkPartition {
    pub fn new(info: PartitionInfo, chunk_index: u8) -> Self {
        Self {
            info,
            chunk_index,
            bytes_written: 0,
            complete: false,
        }
    }

    /// First data LBA = start + 8192 (32 reserved + ~8160 FAT for 4 GiB).
    pub fn data_start_lba(&self) -> u64 {
        const DATA_OFFSET_SECTORS: u64 = 8192;
        self.info.start_lba + DATA_OFFSET_SECTORS
    }

    pub fn max_data_size(&self) -> u64 {
        let total_sectors = self.info.size_sectors();
        const OVERHEAD_SECTORS: u64 = 8192;
        if total_sectors > OVERHEAD_SECTORS {
            (total_sectors - OVERHEAD_SECTORS) * SECTOR_SIZE as u64
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkSet {
    pub chunks: [ChunkPartition; MAX_CHUNK_PARTITIONS],
    pub count: usize,
    pub total_size: u64,
    pub bytes_written: u64,
}

impl ChunkSet {
    pub const fn new() -> Self {
        Self {
            chunks: [ChunkPartition {
                info: PartitionInfo {
                    index: 0,
                    start_lba: 0,
                    end_lba: 0,
                    type_guid: [0; 16],
                    name: [0; 36],
                },
                chunk_index: 0,
                bytes_written: 0,
                complete: false,
            }; MAX_CHUNK_PARTITIONS],
            count: 0,
            total_size: 0,
            bytes_written: 0,
        }
    }

    pub fn add(&mut self, chunk: ChunkPartition) -> DiskResult<()> {
        if self.count >= MAX_CHUNK_PARTITIONS {
            return Err(DiskError::WriteOverflow);
        }
        self.chunks[self.count] = chunk;
        self.count += 1;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<&ChunkPartition> {
        if index < self.count {
            Some(&self.chunks[index])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut ChunkPartition> {
        if index < self.count {
            Some(&mut self.chunks[index])
        } else {
            None
        }
    }
}

impl Default for ChunkSet {
    fn default() -> Self {
        Self::new()
    }
}
