//! On-disk data structures for HelixFS.
//!
//! Every structure is `#[repr(C, packed)]` so it can be written to / read
//! from disk with no padding surprises.  All multi-byte fields are
//! little-endian (x86-64 native byte order).
//!
//! ## Size budget
//!
//! | Structure            | Size    |
//! |----------------------|---------|
//! | `HelixSuperblock`    | 4096 B  |
//! | `LogSegmentHeader`   | 64 B    |
//! | `LogRecordHeader`    | 80 B    |
//! | `IndexNode`          | 4096 B  |
//! | `IndexLeaf`          | 256 B   |
//! | `ExtentEntry`        | 24 B    |
//!
//! One block = 4096 bytes = one x86-64 page.

use core::fmt;

// ═══════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════

/// HelixFS magic bytes (8 bytes).
pub const HELIX_MAGIC: [u8; 8] = *b"HELIXFS1";

/// On-disk format version.  Increment on incompatible layout changes.
pub const HELIX_VERSION: u32 = 1;

/// Block size in bytes (must equal page size for future mmap).
pub const BLOCK_SIZE: u32 = 4096;

/// Shift amount: log2(BLOCK_SIZE).
pub const BLOCK_SHIFT: u32 = 12;

/// Log segment size in blocks (1 MB = 256 blocks).
pub const LOG_SEGMENT_BLOCKS: u64 = 256;

/// Log segment size in bytes.
pub const LOG_SEGMENT_BYTES: u64 = LOG_SEGMENT_BLOCKS * BLOCK_SIZE as u64;

/// Maximum path length in bytes (excluding NUL terminator).
pub const MAX_PATH_LEN: usize = 255;

/// Maximum inline data that fits inside an IndexLeaf.
pub const INLINE_DATA_SIZE: usize = 96;

/// Maximum extents per leaf extent node (determines max file size
/// per indirection level).
pub const EXTENTS_PER_NODE: usize = 168; // 4096 / 24 = 170, minus header

/// B-tree order (keys per internal node).
pub const BTREE_ORDER: usize = 204; // ~4096 / (8 key + 8 child + padding)

/// Maximum file descriptors per process.
pub const MAX_FDS: usize = 64;

/// Maximum mount table entries.
pub const MAX_MOUNTS: usize = 16;

/// Maximum number of snapshots.
pub const MAX_SNAPSHOTS: usize = 256;

/// Reserved block: Superblock A.
pub const SUPERBLOCK_A_BLOCK: u64 = 0;

/// Reserved block: Superblock B.
pub const SUPERBLOCK_B_BLOCK: u64 = 1;

/// Type alias for Log Sequence Number.
pub type Lsn = u64;

/// Type alias for block addresses (4 KiB granularity).
pub type BlockAddr = u64;

/// Sentinel for "no block" / null pointer.
pub const BLOCK_NULL: BlockAddr = 0;

/// Root directory — always inode / index entry #1.
pub const ROOT_DIR_KEY: u64 = 1;

// ═══════════════════════════════════════════════════════════════════════
// Superblock (4096 bytes — one full block)
// ═══════════════════════════════════════════════════════════════════════

/// On-disk superblock.  Two copies exist at blocks 0 and 1.
/// They are written alternately so that at least one is always valid.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HelixSuperblock {
    // ── Identity (offset 0) ─────────────────────────────────────────
    /// Magic: `HELIXFS1`.
    pub magic:             [u8; 8],
    /// On-disk format version.
    pub version:           u32,
    /// Block size in bytes (always 4096).
    pub block_size:        u32,

    // ── Geometry (offset 16) ────────────────────────────────────────
    /// Total blocks on this partition.
    pub total_blocks:      u64,
    /// First block of the log region.
    pub log_start_block:   u64,
    /// Last block of the log region (inclusive).
    pub log_end_block:     u64,
    /// Number of log segment slots.
    pub log_segment_count: u64,
    /// First block of the inode/block bitmaps.
    pub bitmap_start:      u64,
    /// Number of bitmap blocks.
    pub bitmap_blocks:     u64,
    /// First block of the data region.
    pub data_start_block:  u64,
    /// Total data blocks available.
    pub data_block_count:  u64,

    // ── Index root (offset 80) ─────────────────────────────────────
    /// Block address of the current B-tree root (valid at last checkpoint).
    pub index_root_block:  BlockAddr,
    /// B-tree depth (0 = root is a leaf node).
    pub index_depth:       u32,
    /// Padding.
    pub _pad0:             u32,

    // ── Log state (offset 96) ──────────────────────────────────────
    /// Highest fully committed LSN.
    pub committed_lsn:     Lsn,
    /// LSN of the last checkpoint (B-tree was flushed here).
    pub checkpoint_lsn:    Lsn,
    /// Log head segment index (next write position).
    pub log_head_segment:  u64,
    /// Log tail segment index (oldest live data).
    pub log_tail_segment:  u64,
    /// Write offset within the head segment (bytes).
    pub log_head_offset:   u32,
    /// Padding.
    pub _pad1:             u32,

    // ── Volume identity (offset 136) ───────────────────────────────
    /// Random UUID (generated at format time).
    pub uuid:              [u8; 16],
    /// Volume label (null-terminated UTF-8, max 63 chars).
    pub label:             [u8; 64],

    // ── Snapshot bookkeeping (offset 216) ──────────────────────────
    /// Block address of the snapshot table (SnapshotEntry array).
    pub snapshot_table_block: BlockAddr,
    /// Number of named snapshots (0..MAX_SNAPSHOTS).
    pub snapshot_count:    u32,
    pub _pad2:             u32,

    // ── Statistics (offset 232) ────────────────────────────────────
    /// Total blocks currently allocated (data + index + log overhead).
    pub blocks_used:       u64,
    /// Total files (non-dir index entries).
    pub file_count:        u64,
    /// Total directories.
    pub dir_count:         u64,
    /// Creation timestamp (TSC nanoseconds since boot).
    pub created_ns:        u64,
    /// Last mount timestamp.
    pub last_mount_ns:     u64,
    /// Mount count.
    pub mount_count:       u64,

    // ── Integrity (offset 280) ─────────────────────────────────────
    /// CRC32C of bytes [0..280).  Set to 0 during computation.
    pub crc32c:            u32,

    // ── Reserved (offset 284 → 4096) ──────────────────────────────
    pub _reserved:         [u8; 3812],
}

const _ASSERT_SB_SIZE: () = assert!(core::mem::size_of::<HelixSuperblock>() == 4096);

impl HelixSuperblock {
    /// A zeroed superblock (not valid — no magic).
    pub const fn zeroed() -> Self {
        // Safety: all-zeros is a valid bit pattern for every field.
        unsafe { core::mem::zeroed() }
    }

    /// Check magic and CRC.
    pub fn is_valid(&self) -> bool {
        if self.magic != HELIX_MAGIC {
            return false;
        }
        self.verify_crc()
    }

    /// Recompute and store the CRC.
    pub fn update_crc(&mut self) {
        self.crc32c = 0;
        let bytes = unsafe {
            core::slice::from_raw_parts(self as *const _ as *const u8, 288)
        };
        self.crc32c = crate::crc::crc32c(bytes);
    }

    /// Verify the stored CRC matches the computed CRC.
    pub fn verify_crc(&self) -> bool {
        let mut copy = *self;
        copy.crc32c = 0;
        let bytes = unsafe {
            core::slice::from_raw_parts(&copy as *const _ as *const u8, 288)
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

// ═══════════════════════════════════════════════════════════════════════
// Snapshot entry
// ═══════════════════════════════════════════════════════════════════════

/// A named snapshot — just a label + LSN.  Stored in a flat table on disk.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnapshotEntry {
    /// Snapshot name (null-terminated UTF-8, max 63 chars).
    pub name:         [u8; 64],
    /// LSN at the moment of snapshot.
    pub lsn:          Lsn,
    /// Timestamp (TSC nanoseconds).
    pub timestamp_ns: u64,
    /// Index root block at that LSN (for fast mount of snapshot view).
    pub index_root:   BlockAddr,
    /// Reserved.
    pub _pad:         [u8; 40],
}

const _ASSERT_SNAP_SIZE: () = assert!(core::mem::size_of::<SnapshotEntry>() == 128);

// ═══════════════════════════════════════════════════════════════════════
// Log structures
// ═══════════════════════════════════════════════════════════════════════

/// Header at the start of each 1 MB log segment.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LogSegmentHeader {
    /// Magic for segment validation.
    pub magic:          [u8; 4],   // "HLSG"
    /// Explicit padding for alignment.
    pub _pad_magic:     u32,
    /// Segment sequence number (monotonically increasing).
    pub sequence:       u64,
    /// LSN of the first record in this segment.
    pub lsn_start:      Lsn,
    /// Number of complete records in this segment.
    pub record_count:   u32,
    /// Bytes used in this segment (excluding this header).
    pub bytes_used:     u32,
    /// Timestamp of first record (TSC ns).
    pub timestamp_ns:   u64,
    /// CRC32C of this header (fields [0..56), crc set to 0).
    pub crc32c:         u32,
    /// Reserved.
    pub _reserved:      [u8; 20],
}

const _ASSERT_SEGHDR_SIZE: () = assert!(core::mem::size_of::<LogSegmentHeader>() == 64);

/// Segment magic bytes.
pub const LOG_SEGMENT_MAGIC: [u8; 4] = *b"HLSG";

/// Operations that can appear in the log.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOp {
    /// Create or overwrite a file.  Payload = file data.
    Write       = 0x01,
    /// Append data to an existing file.
    Append      = 0x02,
    /// Delete a file or directory (tombstone).
    Delete      = 0x03,
    /// Create a directory (no payload).
    MkDir       = 0x04,
    /// Rename: `old_path_hash` → `new_path_hash`.
    Rename      = 0x05,
    /// Set metadata (flags, timestamps).
    SetMeta     = 0x06,
    /// Dedup reference: payload_crc64 matches an existing block.
    DedupRef    = 0x07,
    /// Transaction begin marker.
    TxBegin     = 0x08,
    /// Transaction commit: atomically applies all records since TxBegin.
    TxCommit    = 0x09,
    /// Transaction abort.
    TxAbort     = 0x0A,
    /// Snapshot label: names a point in the log.
    Snapshot    = 0x0B,
    /// Checkpoint: B-tree root was flushed to disk.
    Checkpoint  = 0x0C,
    /// Truncate a file to a given size.
    Truncate    = 0x0D,
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
            _    => None,
        }
    }
}

/// Fixed-size header preceding every log record.
///
/// Variable-length payload follows immediately after.  Total record
/// size = `size_of::<LogRecordHeader>() + payload_len`, rounded up to
/// 8-byte alignment.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LogRecordHeader {
    /// Log Sequence Number (unique, monotonically increasing).
    pub lsn:            Lsn,
    /// Timestamp from calibrated TSC (nanoseconds since boot).
    pub timestamp_ns:   u64,
    /// Operation type.
    pub op:             u8,
    /// Flags (reserved, must be 0).
    pub flags:          u8,
    /// Padding.
    pub _pad:           [u8; 2],
    /// Total payload length in bytes.
    pub payload_len:    u32,
    /// FNV-1a hash of the full path (for namespace index key).
    pub path_hash:      u64,
    /// CRC64 of the payload (for content dedup).
    pub payload_crc64:  u64,
    /// For Rename: the *new* path hash.  Otherwise 0.
    pub secondary_hash: u64,
    /// For TxCommit: the LSN of the matching TxBegin.  Otherwise 0.
    pub tx_begin_lsn:   Lsn,
    /// CRC32C of this header + payload.  Set to 0 during computation.
    pub record_crc32c:  u32,
    /// Reserved.
    pub _reserved:      u32,
}

const _ASSERT_RECHDR_SIZE: () = assert!(core::mem::size_of::<LogRecordHeader>() == 64);

impl LogRecordHeader {
    /// Total on-disk size of this record (header + payload, 8-byte aligned).
    pub fn total_size(&self) -> u64 {
        let raw = core::mem::size_of::<Self>() as u64 + self.payload_len as u64;
        (raw + 7) & !7 // round up to 8
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Namespace index — B-tree
// ═══════════════════════════════════════════════════════════════════════

/// Flags on an index entry (leaf).
pub mod entry_flags {
    /// Entry is a directory.
    pub const IS_DIR:     u32 = 1 << 0;
    /// Entry has been deleted (tombstone).
    pub const IS_DELETED: u32 = 1 << 1;
    /// Data is stored inline in `inline_data`.
    pub const IS_INLINE:  u32 = 1 << 2;
    /// Entry is a synthetic VFS node (/sys).
    pub const IS_SYS:     u32 = 1 << 3;
    /// Content-addressed dedup is active.
    pub const IS_DEDUP:   u32 = 1 << 4;
}

/// A single entry in a B-tree leaf node.
///
/// 256 bytes.  One block holds 16 entries (4096 / 256).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IndexEntry {
    /// FNV-1a hash of the full path (primary key).
    pub key:            u64,
    /// Full path, null-terminated UTF-8.
    pub path:           [u8; 256],
    /// Bit flags (see `entry_flags`).
    pub flags:          u32,
    /// LSN of the log record containing the latest data.
    pub lsn:            Lsn,
    /// File size in bytes (0 for directories).
    pub size:           u64,
    /// Number of direct children (directories only).
    pub child_count:    u32,
    /// Padding.
    pub _pad0:          u32,
    /// Creation timestamp (TSC ns).
    pub created_ns:     u64,
    /// Last modification timestamp.
    pub modified_ns:    u64,
    /// Number of prior versions in the log.
    pub version_count:  u32,
    /// LSN of the first version of this path.
    pub first_lsn:      Lsn,
    /// Padding.
    pub _pad1:          u32,
    /// Inline data for small files (< 96 bytes).
    pub inline_data:    [u8; INLINE_DATA_SIZE],
    /// Block address of extent tree root (large files).
    pub extent_root:    BlockAddr,
    /// CRC64 of current content (for dedup).
    pub content_crc64:  u64,
    /// CRC32C of this entry.
    pub crc32c:         u32,
    /// Reserved.
    pub _reserved:      [u8; 60],
}

// Target: 512 bytes per entry → 8 entries per block
const _ASSERT_ENTRY_SIZE: () = assert!(core::mem::size_of::<IndexEntry>() == 512);

/// B-tree internal node.  Fits in one block (4096 bytes).
///
/// Layout: `[header (32 bytes)] [keys: u64 × ORDER] [children: u64 × (ORDER+1)]`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BTreeNodeHeader {
    /// Node type marker: 0x01 = internal, 0x02 = leaf.
    pub node_type:    u8,
    /// Padding.
    pub _pad:         [u8; 3],
    /// Number of keys currently stored.
    pub key_count:    u32,
    /// Block address of this node (self-reference for validation).
    pub self_block:   BlockAddr,
    /// CRC32C of the entire block.
    pub crc32c:       u32,
    /// Reserved.
    pub _reserved:    [u8; 12],
}

const _ASSERT_BTREE_HDR: () = assert!(core::mem::size_of::<BTreeNodeHeader>() == 32);

/// Node type constants.
pub const NODE_INTERNAL: u8 = 0x01;
pub const NODE_LEAF:     u8 = 0x02;

/// Internal node: header + keys[ORDER] + children[ORDER+1].
///
/// With ORDER=253: header(32) + keys(253×8=2024) + children(254×8=2032) = 4088 ≤ 4096.
pub const INTERNAL_ORDER: usize = 253;

/// Leaf node: header + entries.
///
/// With 512-byte entries: (4096 - 32) / 512 = 7 entries per leaf block.
pub const LEAF_ENTRIES_PER_BLOCK: usize = 7;

// ═══════════════════════════════════════════════════════════════════════
// Extent tree
// ═══════════════════════════════════════════════════════════════════════

/// A single extent: a contiguous run of data blocks.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExtentEntry {
    /// Logical block offset within the file.
    pub logical_block:  u64,
    /// Physical block address on disk.
    pub physical_block: BlockAddr,
    /// Number of contiguous blocks.
    pub block_count:    u32,
    /// Reserved.
    pub _reserved:      u32,
}

const _ASSERT_EXTENT_SIZE: () = assert!(core::mem::size_of::<ExtentEntry>() == 24);

/// An extent node header (first 16 bytes of a block).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExtentNodeHeader {
    /// 0x01 = leaf (entries are ExtentEntry), 0x02 = internal (entries are children).
    pub node_type:  u8,
    pub _pad:       [u8; 3],
    /// Number of entries in this node.
    pub count:      u32,
    /// CRC32C of the block.
    pub crc32c:     u32,
    pub _reserved:  u32,
}

const _ASSERT_EXTHDR_SIZE: () = assert!(core::mem::size_of::<ExtentNodeHeader>() == 16);

/// Extents per leaf = (4096 - 16) / 24 = 170.
pub const EXTENTS_PER_LEAF: usize = 170;

// ═══════════════════════════════════════════════════════════════════════
// VFS types — in-memory only, not on disk
// ═══════════════════════════════════════════════════════════════════════

/// Open mode flags.
pub mod open_flags {
    /// Open for reading.
    pub const O_READ:    u32 = 1 << 0;
    /// Open for writing.
    pub const O_WRITE:   u32 = 1 << 1;
    /// Create if not exists.
    pub const O_CREATE:  u32 = 1 << 2;
    /// Truncate on open.
    pub const O_TRUNC:   u32 = 1 << 3;
    /// Append mode (all writes go to end).
    pub const O_APPEND:  u32 = 1 << 4;
    /// Open a directory for iteration.
    pub const O_DIR:     u32 = 1 << 5;
    /// Open at a specific LSN (temporal read).
    pub const O_AT_LSN:  u32 = 1 << 6;
}

/// A file descriptor — per-process, in-memory only.
#[derive(Clone, Copy, Debug)]
pub struct FileDescriptor {
    /// Index entry key (path hash) this fd refers to.
    pub key:          u64,
    /// Open flags.
    pub flags:        u32,
    /// Current seek offset in bytes.
    pub offset:       u64,
    /// Mount table index (which filesystem instance).
    pub mount_idx:    u8,
    /// Padding.
    pub _pad:         [u8; 3],
    /// For O_AT_LSN: the pinned LSN for temporal reads.
    pub pinned_lsn:   Lsn,
}

impl FileDescriptor {
    pub const fn empty() -> Self {
        FileDescriptor {
            key: 0,
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
pub struct FileStat {
    /// Full path hash.
    pub key:           u64,
    /// File size in bytes.
    pub size:          u64,
    /// Is directory?
    pub is_dir:        bool,
    /// Created timestamp (TSC ns).
    pub created_ns:    u64,
    /// Modified timestamp (TSC ns).
    pub modified_ns:   u64,
    /// Number of prior versions.
    pub version_count: u32,
    /// Current LSN.
    pub lsn:           Lsn,
    /// First LSN (creation).
    pub first_lsn:     Lsn,
    /// Entry flags.
    pub flags:         u32,
}

// ═══════════════════════════════════════════════════════════════════════
// Syscall numbers for filesystem operations
// ═══════════════════════════════════════════════════════════════════════

/// `open(path_ptr, path_len, flags) → fd`
pub const SYS_OPEN:     u64 = 10;
/// `close(fd) → 0`
pub const SYS_CLOSE:    u64 = 11;
/// `seek(fd, offset, whence) → new_offset`
pub const SYS_SEEK:     u64 = 12;
/// `stat(path_ptr, path_len, stat_buf_ptr) → 0`
pub const SYS_STAT:     u64 = 13;
/// `readdir(fd, entry_buf_ptr, max_entries) → count`
pub const SYS_READDIR:  u64 = 14;
/// `mkdir(path_ptr, path_len) → 0`
pub const SYS_MKDIR:    u64 = 15;
/// `unlink(path_ptr, path_len) → 0`
pub const SYS_UNLINK:   u64 = 16;
/// `rename(old_ptr, old_len, new_ptr, new_len) → 0`
pub const SYS_RENAME:   u64 = 17;
/// `truncate(fd, new_size) → 0`
pub const SYS_TRUNCATE: u64 = 18;
/// `sync() → 0` — flush all pending writes and checkpoint.
pub const SYS_SYNC:     u64 = 19;
/// `snapshot(name_ptr, name_len) → snapshot_id`
pub const SYS_SNAPSHOT: u64 = 20;
/// `versions(path_ptr, path_len, buf_ptr, max) → count`
pub const SYS_VERSIONS: u64 = 21;

/// SEEK whence constants.
pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;
