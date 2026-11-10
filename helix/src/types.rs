//! HelixFS on-disk data structures. `#[repr(C)]`, little-endian, 4 KiB blocks.

use crate::error::HelixError;
use core::fmt;

pub const HELIX_MAGIC: [u8; 8] = *b"HELIXFS1";

/// Layout version. v1: path_hash only. v2: full path in payload. v3: extent
/// Write records are flagged out-of-band (`rec_flags::IS_EXTENT`) instead of by
/// an in-band marker byte that could collide with inline user data.
pub const HELIX_VERSION: u32 = 3;

pub const BLOCK_SIZE: u32 = 4096;
pub const BLOCK_SHIFT: u32 = 12;

pub const LOG_SEGMENT_BLOCKS: u64 = 256;
pub const LOG_SEGMENT_BYTES: u64 = LOG_SEGMENT_BLOCKS * BLOCK_SIZE as u64;

pub const MAX_PATH_LEN: usize = 255;

/// Bytes of file data inlined in an `IndexEntry`.
pub const INLINE_DATA_SIZE: usize = 96;

pub const SUPERBLOCK_A_BLOCK: u64 = 0;
pub const SUPERBLOCK_B_BLOCK: u64 = 1;

pub type Lsn = u64;
pub type BlockAddr = u64;

pub const BLOCK_NULL: BlockAddr = u64::MAX;

/// 4 KiB superblock. Two copies (block 0 / 1) written alternately so at
/// least one is always valid.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HelixSuperblock {
    pub magic: [u8; 8],
    pub version: u32,
    pub block_size: u32,

    pub total_blocks: u64,
    pub log_start_block: u64,
    pub log_end_block: u64,
    pub log_segment_count: u64,
    pub bitmap_start: u64,
    pub bitmap_blocks: u64,
    pub data_start_block: u64,
    pub data_block_count: u64,

    /// B-tree root at last checkpoint.
    pub index_root_block: BlockAddr,
    /// Blocks in the index part of the checkpoint.
    pub index_depth: u32,
    /// `index_flags` bits.
    pub index_flags: u32,

    pub committed_lsn: Lsn,
    /// LSN at which the B-tree was last flushed.
    pub checkpoint_lsn: Lsn,
    pub log_head_segment: u64,
    pub log_tail_segment: u64,
    pub log_head_offset: u32,
    pub _pad1: u32,

    pub uuid: [u8; 16],
    /// Null-terminated UTF-8, max 63 chars.
    pub label: [u8; 64],

    pub snapshot_table_block: BlockAddr,
    pub snapshot_count: u32,
    pub _pad2: u32,

    pub blocks_used: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub created_ns: u64,
    pub last_mount_ns: u64,
    pub mount_count: u64,

    /// CRC32C of the whole 4 KiB block with this field zeroed.
    pub crc32c: u32,

    /// Live entries serialized to the on-disk index checkpoint region
    /// (`index_root_block` for `index_depth` blocks).
    pub index_entry_count: u32,

    pub _reserved: [u8; 3808],
}

const _ASSERT_SB_SIZE: () = assert!(core::mem::size_of::<HelixSuperblock>() == 4096);

impl HelixSuperblock {
    pub const fn zeroed() -> Self {
        // SAFETY: all-zeros is valid for every field.
        unsafe { core::mem::zeroed() }
    }

    pub fn is_valid(&self) -> bool {
        if self.magic != HELIX_MAGIC {
            return false;
        }
        self.verify_crc()
    }

    /// Reject a checksum-valid superblock whose geometry cannot be trusted.
    pub fn validate_geometry(&self) -> Result<(), HelixError> {
        let bad = Err(HelixError::NoValidSuperblock);
        if self.block_size != BLOCK_SIZE {
            return Err(HelixError::InvalidBlockSize);
        }
        let log_blocks = match self.log_segment_count.checked_mul(LOG_SEGMENT_BLOCKS) {
            Some(n) if n != 0 => n,
            _ => return bad,
        };
        if self.log_start_block != 2
            || self.log_start_block.checked_add(log_blocks) != self.log_end_block.checked_add(1)
            || self.bitmap_start != self.log_end_block + 1
            || self.bitmap_start.checked_add(self.bitmap_blocks) != Some(self.data_start_block)
            || self
                .data_start_block
                .checked_add(self.data_block_count)
                .map_or(true, |end| end > self.total_blocks)
        {
            return bad;
        }
        let seg_hdr = core::mem::size_of::<LogSegmentHeader>() as u32;
        if self.log_head_segment >= self.log_segment_count
            || self.log_tail_segment >= self.log_segment_count
            || self.log_head_offset < seg_hdr
            || self.log_head_offset as u64 > LOG_SEGMENT_BYTES
            || self.log_head_offset % 8 != 0
            || self.committed_lsn == u64::MAX
            || self.checkpoint_lsn > self.committed_lsn
        {
            return bad;
        }
        if self.index_flags & !index_flags::ROOT_IS_NODE != 0 {
            return bad;
        }
        if self.index_flags & index_flags::ROOT_IS_NODE != 0 {
            if self.index_root_block >= self.data_block_count
                || self.index_depth == 0
                || self.index_entry_count as u64 > self.index_depth as u64 * 8
            {
                return bad;
            }
        } else if self.index_root_block != BLOCK_NULL {
            let snap_blocks = (self.snapshot_count as u64).div_ceil(8);
            let end = self
                .index_root_block
                .checked_add(self.index_depth as u64)
                .and_then(|e| e.checked_add(snap_blocks));
            if self.index_depth == 0
                || end.map_or(true, |e| e > self.data_block_count)
                || self.index_entry_count as u64 > self.index_depth as u64 * 8
            {
                return bad;
            }
            if self.snapshot_count != 0
                && self.snapshot_table_block != self.index_root_block + self.index_depth as u64
            {
                return bad;
            }
        } else if self.snapshot_count != 0 {
            return bad;
        }
        Ok(())
    }

    pub fn update_crc(&mut self) {
        self.crc32c = 0;
        // SAFETY: HelixSuperblock is #[repr(C)] and sized exactly BLOCK_SIZE
        // (asserted above), so the whole struct can be viewed as a byte slice.
        let bytes = unsafe {
            core::slice::from_raw_parts(self as *const _ as *const u8, BLOCK_SIZE as usize)
        };
        self.crc32c = crate::crc::crc32c(bytes);
    }

    pub fn verify_crc(&self) -> bool {
        let mut copy = *self;
        copy.crc32c = 0;
        // SAFETY: same as update_crc — repr(C) struct exactly BLOCK_SIZE bytes.
        let bytes = unsafe {
            core::slice::from_raw_parts(&copy as *const _ as *const u8, BLOCK_SIZE as usize)
        };
        crate::crc::crc32c(bytes) == self.crc32c
    }
}

impl fmt::Debug for HelixSuperblock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelixSuperblock")
            .field("version", &self.version)
            .field("total_blocks", &self.total_blocks)
            .field("committed_lsn", &self.committed_lsn)
            .field("checkpoint_lsn", &self.checkpoint_lsn)
            .field("blocks_used", &self.blocks_used)
            .finish()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LogSegmentHeader {
    pub magic: [u8; 4], // "HLSG"
    pub _pad_magic: u32,
    pub sequence: u64,
    pub lsn_start: Lsn,
    pub record_count: u32,
    /// Bytes used after this 64-byte header.
    pub bytes_used: u32,
    pub timestamp_ns: u64,
    /// CRC32C of bytes [0..56), with this field zeroed.
    pub crc32c: u32,
    pub _reserved: [u8; 20],
}

const _ASSERT_SEGHDR_SIZE: () = assert!(core::mem::size_of::<LogSegmentHeader>() == 64);

pub const LOG_SEGMENT_MAGIC: [u8; 4] = *b"HLSG";

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOp {
    Write = 0x01,
    Append = 0x02,
    Delete = 0x03,
    MkDir = 0x04,
    Rename = 0x05,
    SetMeta = 0x06,
    /// Payload_crc64 matches an existing block.
    DedupRef = 0x07,
    TxBegin = 0x08,
    /// Atomically applies records since TxBegin.
    TxCommit = 0x09,
    TxAbort = 0x0A,
    Snapshot = 0x0B,
    /// B-tree root flushed to disk.
    Checkpoint = 0x0C,
    Truncate = 0x0D,
}

impl LogOp {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Write),
            0x02 => Some(Self::Append),
            0x03 => Some(Self::Delete),
            0x04 => Some(Self::MkDir),
            0x05 => Some(Self::Rename),
            0x06 => Some(Self::SetMeta),
            0x07 => Some(Self::DedupRef),
            0x08 => Some(Self::TxBegin),
            0x09 => Some(Self::TxCommit),
            0x0A => Some(Self::TxAbort),
            0x0B => Some(Self::Snapshot),
            0x0C => Some(Self::Checkpoint),
            0x0D => Some(Self::Truncate),
            _ => None,
        }
    }
}

/// Fixed-size record header; variable-length payload follows immediately.
/// Total record size = `size_of::<Self>() + payload_len`, rounded up to 8.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LogRecordHeader {
    pub lsn: Lsn,
    /// TSC nanoseconds since boot.
    pub timestamp_ns: u64,
    pub op: u8,
    pub flags: u8,
    pub _pad: [u8; 2],
    pub payload_len: u32,
    /// FNV-1a of the full path; namespace index key.
    pub path_hash: u64,
    /// Payload CRC64; used for dedup.
    pub payload_crc64: u64,
    /// New path hash on Rename, else 0.
    pub secondary_hash: u64,
    /// Matching TxBegin LSN on TxCommit, else 0.
    pub tx_begin_lsn: Lsn,
    /// CRC32C of header + payload with this field zeroed.
    pub record_crc32c: u32,
    pub _reserved: u32,
}

const _ASSERT_RECHDR_SIZE: () = assert!(core::mem::size_of::<LogRecordHeader>() == 64);

impl LogRecordHeader {
    /// On-disk size: header + payload, rounded up to 8 bytes.
    pub fn total_size(&self) -> u64 {
        let raw = core::mem::size_of::<Self>() as u64 + self.payload_len as u64;
        (raw + 7) & !7
    }
}

/// `HelixSuperblock.index_flags` bits.
pub mod index_flags {
    /// `index_root_block` is an extent node listing the checkpoint's runs
    /// (a fragmented checkpoint) instead of the first block of one run.
    pub const ROOT_IS_NODE: u32 = 1 << 0;
}

/// `LogRecordHeader.flags` bits.
pub mod rec_flags {
    /// Write payload is an extent descriptor, not inline data. Authoritative —
    /// classification must never inspect payload bytes.
    pub const IS_EXTENT: u8 = 1 << 0;
}

pub mod entry_flags {
    pub const IS_DIR: u32 = 1 << 0;
    pub const IS_DELETED: u32 = 1 << 1;
    pub const IS_INLINE: u32 = 1 << 2;
    /// `extent_root` addresses an extent-node block, not a contiguous run.
    pub const IS_EXTENT_NODE: u32 = 1 << 5;
    /// `content_crc64` covers the extent data and is verified on read.
    pub const HAS_CONTENT_CRC: u32 = 1 << 6;
    /// Snapshot-table marker: only `lsn` is meaningful.
    pub const IS_SNAPSHOT_MARK: u32 = 1 << 7;
}

/// Discriminates an extent Write payload's residency. Stored as the first byte
/// of the extent descriptor `[kind: u8][file_size: u64][extent_root: u64][crc64: u64]`
/// (the trailing crc is absent in records written before it existed).
pub mod extent_kind {
    /// `extent_root` is the first of `ceil(size/BLOCK)` physically contiguous blocks.
    pub const CONTIGUOUS: u8 = 1;
    /// `extent_root` is an extent-node block listing every run.
    pub const NODE: u8 = 2;
}

/// 512-byte B-tree leaf entry; 8 entries per 4 KiB block.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IndexEntry {
    /// FNV-1a of the full path; primary key.
    pub key: u64,
    pub path: [u8; 256],
    pub flags: u32,
    /// LSN of the record carrying the latest data.
    pub lsn: Lsn,
    pub size: u64,
    /// Direct children; directories only.
    pub child_count: u32,
    pub _pad0: u32,
    pub created_ns: u64,
    pub modified_ns: u64,
    pub version_count: u32,
    pub first_lsn: Lsn,
    pub _pad1: u32,
    pub inline_data: [u8; INLINE_DATA_SIZE],
    pub extent_root: BlockAddr,
    /// CRC64 of current content; for dedup.
    pub content_crc64: u64,
    pub crc32c: u32,
    pub _reserved: [u8; 60],
}

const _ASSERT_ENTRY_SIZE: () = assert!(core::mem::size_of::<IndexEntry>() == 512);

/// A single extent: a contiguous run of data blocks.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExtentEntry {
    /// Logical block offset within the file.
    pub logical_block: u64,
    /// Physical block address on disk.
    pub physical_block: BlockAddr,
    /// Number of contiguous blocks.
    pub block_count: u32,
    /// Reserved.
    pub _reserved: u32,
}

const _ASSERT_EXTENT_SIZE: () = assert!(core::mem::size_of::<ExtentEntry>() == 24);

/// An extent node header (first 16 bytes of a block).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExtentNodeHeader {
    /// 0x01 = leaf (entries are ExtentEntry), 0x02 = internal (entries are children).
    pub node_type: u8,
    pub _pad: [u8; 3],
    /// Number of entries in this node.
    pub count: u32,
    /// CRC32C of the block.
    pub crc32c: u32,
    pub _reserved: u32,
}

const _ASSERT_EXTHDR_SIZE: () = assert!(core::mem::size_of::<ExtentNodeHeader>() == 16);

/// Extents per leaf = (4096 - 16) / 24 = 170.
pub const EXTENTS_PER_LEAF: usize = 170;

/// Open mode flags.
///
/// Canonical open flags live in `morpheus_foundation::flags::open_flags`; this
/// re-export keeps the `helix::types::open_flags::*` path while single-sourcing
/// the values.
pub use morpheus_foundation::flags::open_flags;

// Canonical kernel↔userland FFI types, syscall numbers, and seek whence all live
// in morpheus-foundation — the single source of truth. Re-exported (not
// re-declared) so the values can never desync.
pub use morpheus_foundation::syscall_abi::{
    SEEK_CUR, SEEK_END, SEEK_SET, SYS_CLOSE, SYS_MKDIR, SYS_OPEN, SYS_READDIR, SYS_RENAME,
    SYS_SEEK, SYS_SNAPSHOT, SYS_STAT, SYS_SYNC, SYS_TRUNCATE, SYS_UNLINK, SYS_VERSIONS,
};
pub use morpheus_foundation::types::{DirEntry, FileStat};
