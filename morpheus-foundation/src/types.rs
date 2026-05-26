//! `#[repr(C)]` types forming the stable ABI between the helix surface and libmorpheus.

/// `stat(path, &mut buf)` writes this into `buf`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FileStat {
    /// Full-path hash (helix key).
    pub key: u64,
    pub size: u64,
    pub is_dir: bool,
    /// TSC nanoseconds since boot.
    pub created_ns: u64,
    /// TSC nanoseconds since boot.
    pub modified_ns: u64,
    pub version_count: u32,
    /// Helix log sequence number.
    pub lsn: u64,
    /// Creation LSN.
    pub first_lsn: u64,
    /// Entry flags.
    pub flags: u32,
}

/// One entry from `readdir(fd, &mut buf, max_entries)`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct DirEntry {
    /// Filename — last path component only. Length in `name_len`.
    pub name: [u8; 256],
    pub name_len: u16,
    pub is_dir: bool,
    /// 0 for directories.
    pub size: u64,
    pub modified_ns: u64,
    pub version_count: u32,
}
