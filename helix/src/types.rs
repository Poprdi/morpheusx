//! HelixFS on-disk data structures. `#[repr(C)]`, little-endian, 4 KiB blocks.

use core::fmt;

pub const HELIX_MAGIC: [u8; 8] = *b"HELIXFS1";

/// Layout version. v1 stored only `path_hash` (no replay); v2 carries full path.
pub const HELIX_VERSION: u32 = 2;

pub const BLOCK_SIZE: u32 = 4096;
pub const BLOCK_SHIFT: u32 = 12;

pub const LOG_SEGMENT_BLOCKS: u64 = 256;
pub const LOG_SEGMENT_BYTES: u64 = LOG_SEGMENT_BLOCKS * BLOCK_SIZE as u64;

pub const MAX_PATH_LEN: usize = 255;

/// Bytes of file data inlined in an `IndexEntry`.
pub const INLINE_DATA_SIZE: usize = 96;

/// 4096 / 24 = 170 entries; leave room for header.
pub const EXTENTS_PER_NODE: usize = 168;

/// ~4096 / (8 key + 8 child + padding).
pub const BTREE_ORDER: usize = 204;

pub const MAX_FDS: usize = 64;
pub const MAX_MOUNTS: usize = 16;
pub const MAX_SNAPSHOTS: usize = 256;

pub const SUPERBLOCK_A_BLOCK: u64 = 0;
pub const SUPERBLOCK_B_BLOCK: u64 = 1;

pub type Lsn = u64;
pub type BlockAddr = u64;

pub const BLOCK_NULL: BlockAddr = u64::MAX;

pub const ROOT_DIR_KEY: u64 = 1;

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
    /// 0 = root is a leaf.
    pub index_depth: u32,
    pub _pad0: u32,

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

    /// CRC32C of bytes [0..280) with this field zeroed during computation.
    pub crc32c: u32,

    pub _reserved: [u8; 3812],
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

    pub fn update_crc(&mut self) {
        self.crc32c = 0;
        let bytes = unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, 288) };
        self.crc32c = crate::crc::crc32c(bytes);
    }

    pub fn verify_crc(&self) -> bool {
        let mut copy = *self;
        copy.crc32c = 0;
        let bytes = unsafe { core::slice::from_raw_parts(&copy as *const _ as *const u8, 288) };
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

/// Named snapshot: label + LSN, stored in a flat on-disk table.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnapshotEntry {
    pub name: [u8; 64],
    pub lsn: Lsn,
    pub timestamp_ns: u64,
    /// B-tree root at this LSN for fast snapshot mount.
    pub index_root: BlockAddr,
    pub _pad: [u8; 40],
}

const _ASSERT_SNAP_SIZE: () = assert!(core::mem::size_of::<SnapshotEntry>() == 128);

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

pub mod entry_flags {
    pub const IS_DIR: u32 = 1 << 0;
    pub const IS_DELETED: u32 = 1 << 1;
    pub const IS_INLINE: u32 = 1 << 2;
    /// Synthetic VFS node (e.g. /sys).
    pub const IS_SYS: u32 = 1 << 3;
    pub const IS_DEDUP: u32 = 1 << 4;
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

/// B-tree internal node header. Block layout:
/// `[header(32)][keys: u64 × ORDER][children: u64 × (ORDER+1)]`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BTreeNodeHeader {
    /// 0x01 internal, 0x02 leaf.
    pub node_type: u8,
    pub _pad: [u8; 3],
    pub key_count: u32,
    /// Self-block; validates the block was read from where we expected.
    pub self_block: BlockAddr,
    pub crc32c: u32,
    pub _reserved: [u8; 12],
}

const _ASSERT_BTREE_HDR: () = assert!(core::mem::size_of::<BTreeNodeHeader>() == 32);

pub const NODE_INTERNAL: u8 = 0x01;
pub const NODE_LEAF: u8 = 0x02;

/// Internal node: header + keys[ORDER] + children[ORDER+1].
///
/// With ORDER=253: header(32) + keys(253×8=2024) + children(254×8=2032) = 4088 ≤ 4096.
pub const INTERNAL_ORDER: usize = 253;

/// Leaf node: header + entries.
///
/// With 512-byte entries: (4096 - 32) / 512 = 7 entries per leaf block.
pub const LEAF_ENTRIES_PER_BLOCK: usize = 7;

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
/// MUST match the constants in `libmorpheus/src/fs.rs` exactly — these
/// values cross the syscall ABI boundary verbatim.
pub mod open_flags {
    /// Open for reading.
    pub const O_READ: u32 = 1 << 0; // 0x01
    /// Open for writing.
    pub const O_WRITE: u32 = 1 << 1; // 0x02
    pub const O_CREATE: u32 = 1 << 2; // 0x04
                                      // bit 3 (0x08) reserved — not used in libmorpheus ABI
    /// Truncate on open.
    pub const O_TRUNC: u32 = 1 << 4; // 0x10  (matches libmorpheus)
    pub const O_APPEND: u32 = 1 << 5; // 0x20  (matches libmorpheus)
    /// Open a directory for iteration.
    pub const O_DIR: u32 = 1 << 6; // 0x40
    /// Open at a specific LSN (temporal read).
    pub const O_AT_LSN: u32 = 1 << 7; // 0x80
                                      // ── Pipe markers (kernel-internal, not user-visible) ─────────
    /// This fd is the read end of a kernel pipe.
    pub const O_PIPE_READ: u32 = 1 << 8;
    /// This fd is the write end of a kernel pipe.
    pub const O_PIPE_WRITE: u32 = 1 << 9;
}

/// A file descriptor — per-process, in-memory only.
#[derive(Clone, Copy, Debug)]
pub struct FileDescriptor {
    /// Index entry key (path hash) this fd refers to.
    pub key: u64,
    /// Full path for safe index lookups (avoids hash-collision issues).
    pub path: [u8; 256],
    /// Open flags.
    pub flags: u32,
    /// Current seek offset in bytes.
    pub offset: u64,
    /// Mount table index (which filesystem instance).
    pub mount_idx: u8,
    /// Padding.
    pub _pad: [u8; 3],
    /// For O_AT_LSN: the pinned LSN for temporal reads.
    pub pinned_lsn: Lsn,
}

impl FileDescriptor {
    pub const fn empty() -> Self {
        FileDescriptor {
            key: 0,
            path: [0u8; 256],
            flags: 0,
            offset: 0,
            mount_idx: 0,
            _pad: [0; 3],
            pinned_lsn: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.flags != 0
    }

    pub fn is_readable(&self) -> bool {
        self.flags & open_flags::O_READ != 0
    }

    pub fn is_writable(&self) -> bool {
        self.flags & open_flags::O_WRITE != 0
    }
}

/// Directory entry returned by readdir.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DirEntry {
    /// Filename (not full path — just the last component).
    pub name: [u8; 256],
    /// Name length in bytes.
    pub name_len: u16,
    /// Is this a directory?
    pub is_dir: bool,
    /// File size (0 for directories).
    pub size: u64,
    /// Last modification timestamp (TSC ns).
    pub modified_ns: u64,
    /// Version count.
    pub version_count: u32,
}

/// File stat information.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FileStat {
    /// Full path hash.
    pub key: u64,
    /// File size in bytes.
    pub size: u64,
    /// Is directory?
    pub is_dir: bool,
    /// Created timestamp (TSC ns).
    pub created_ns: u64,
    /// Modified timestamp (TSC ns).
    pub modified_ns: u64,
    /// Number of prior versions.
    pub version_count: u32,
    /// Current LSN.
    pub lsn: Lsn,
    /// First LSN (creation).
    pub first_lsn: Lsn,
    /// Entry flags.
    pub flags: u32,
}

/// `open(path_ptr, path_len, flags) → fd`
pub const SYS_OPEN: u64 = 10;
/// `close(fd) → 0`
pub const SYS_CLOSE: u64 = 11;
/// `seek(fd, offset, whence) → new_offset`
pub const SYS_SEEK: u64 = 12;
/// `stat(path_ptr, path_len, stat_buf_ptr) → 0`
pub const SYS_STAT: u64 = 13;
/// `readdir(fd, entry_buf_ptr, max_entries) → count`
pub const SYS_READDIR: u64 = 14;
/// `mkdir(path_ptr, path_len) → 0`
pub const SYS_MKDIR: u64 = 15;
/// `unlink(path_ptr, path_len) → 0`
pub const SYS_UNLINK: u64 = 16;
/// `rename(old_ptr, old_len, new_ptr, new_len) → 0`
pub const SYS_RENAME: u64 = 17;
/// `truncate(fd, new_size) → 0`
pub const SYS_TRUNCATE: u64 = 18;
/// `sync() → 0` — flush all pending writes and checkpoint.
pub const SYS_SYNC: u64 = 19;
/// `snapshot(name_ptr, name_len) → snapshot_id`
pub const SYS_SNAPSHOT: u64 = 20;
/// `versions(path_ptr, path_len, buf_ptr, max) → count`
pub const SYS_VERSIONS: u64 = 21;

pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;
