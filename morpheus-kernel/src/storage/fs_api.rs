//! The filesystem-backend seam (spec §4): `FsBackend` trait, `VfsError`, and per-fd state.

use alloc::vec::Vec;
use morpheus_block_types::RawBlockDevice;
use morpheus_foundation::storage::FD_COOKIE_LEN;
use morpheus_foundation::types::{DirEntry, FileStat};

/// One canonical FS error (spec §4). Each backend maps its private error
/// (`HelixError`/`Fat32Error`) into this; the subsystem owns the single
/// `VfsError → errno` table (see `mod::vfs_err_to_errno`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotEmpty,
    BadFd,
    TooManyOpen,
    ReadOnly,
    NoSpace,
    Io,
    Inval,
    Unsupported,
    Perm,
    NameTooLong,
    CrossDevice,
    Busy,
    NoDev,
}

/// What a backend can do. `open(O_WRITE)` against `writable:false` is rejected up
/// front (→ `ReadOnly`); the capability-gated trait methods default to
/// `Unsupported` so a backend cannot silently lie about a missing feature.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsCapabilities {
    pub writable: bool,
    pub resizable: bool,
    pub snapshots: bool,
    pub versions: bool,
}

/// Per-fd state owned by `FdTable`. `cookie` is backend-private; reap = drop table (spec §4).
#[derive(Clone, Copy)]
pub struct FdState {
    pub mount_id: u64,
    pub flags: u32,
    pub offset: u64,
    pub path: [u8; 256],
    pub path_len: u16,
    pub cookie: [u8; FD_COOKIE_LEN],
    /// Set by `MNT_FORCE` umount revoke; a revoked fd's next op → `BadFd`.
    pub revoked: bool,
    in_use: bool,
}

impl FdState {
    pub const fn empty() -> Self {
        Self {
            mount_id: 0,
            flags: 0,
            offset: 0,
            path: [0u8; 256],
            path_len: 0,
            cookie: [0u8; FD_COOKIE_LEN],
            revoked: false,
            in_use: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.in_use
    }

    pub fn path_str(&self) -> &str {
        let len = (self.path_len as usize).min(self.path.len());
        core::str::from_utf8(&self.path[..len]).unwrap_or("")
    }
}

/// Per-process fd table. Bounded (DoS limit, not an array-as-capacity decision)
/// and **not `Copy`** — duplicating one across processes must be explicit, so the
/// per-mount `open_fds` refcount stays accurate. Const-constructible for the
/// `PROCESS_TABLE` static.
pub const FD_TABLE_LEN: usize = 64;

#[derive(Clone)]
pub struct FdTable {
    slots: [FdState; FD_TABLE_LEN],
}

impl FdTable {
    pub const fn new() -> Self {
        Self {
            slots: [FdState::empty(); FD_TABLE_LEN],
        }
    }

    /// Lowest free fd at or above 3, or `None` (→ `TooManyOpen`). fds 0/1/2 are
    /// reserved for stdin/stdout/stderr and are never handed to files or pipes;
    /// allocating them would let a pipe/file hijack stdio, since the read/write
    /// dispatch consults this table before the stdio fallback.
    pub fn alloc(&mut self) -> Option<usize> {
        (3..FD_TABLE_LEN).find(|&i| !self.slots[i].in_use)
    }

    pub fn get(&self, fd: usize) -> Option<&FdState> {
        let s = self.slots.get(fd)?;
        if s.in_use {
            Some(s)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, fd: usize) -> Option<&mut FdState> {
        let s = self.slots.get_mut(fd)?;
        if s.in_use {
            Some(s)
        } else {
            None
        }
    }

    /// Install `state` at `fd`, marking it live. Caller bumps the mount refcount.
    pub fn set(&mut self, fd: usize, mut state: FdState) -> bool {
        match self.slots.get_mut(fd) {
            Some(slot) => {
                state.in_use = true;
                *slot = state;
                true
            },
            None => false,
        }
    }

    /// Free `fd`, returning its `mount_id` if it was open (caller decrements the
    /// mount's `open_fds`).
    pub fn free(&mut self, fd: usize) -> Option<u64> {
        let slot = self.slots.get_mut(fd)?;
        if !slot.in_use {
            return None;
        }
        let mount_id = slot.mount_id;
        *slot = FdState::empty();
        Some(mount_id)
    }

    /// Live `(fd, &FdState)` pairs — used by reap to close everything.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &FdState)> {
        self.slots.iter().enumerate().filter(|(_, s)| s.in_use)
    }
}

impl Default for FdTable {
    fn default() -> Self {
        Self::new()
    }
}

/// `open` result: the cookie the backend wants persisted, plus whether the
/// resolved object is a directory (the VFS uses this to gate read/write).
pub struct OpenFile {
    pub cookie: [u8; FD_COOKIE_LEN],
    pub is_dir: bool,
}

/// FS-backend contract (spec §4). Capability-gated methods default to `Unsupported`.
pub trait FsBackend {
    fn capabilities(&self) -> FsCapabilities;

    fn open(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        flags: u32,
        ts: u64,
    ) -> Result<OpenFile, VfsError>;

    fn read(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &FdState,
        buf: &mut [u8],
    ) -> Result<usize, VfsError>;

    fn stat(&mut self, dev: &mut RawBlockDevice, path: &str) -> Result<FileStat, VfsError>;

    fn readdir(&mut self, dev: &mut RawBlockDevice, path: &str) -> Result<Vec<DirEntry>, VfsError>;

    fn close(&mut self, _dev: &mut RawBlockDevice, _f: &FdState) -> Result<(), VfsError> {
        Ok(())
    }

    fn write(
        &mut self,
        _dev: &mut RawBlockDevice,
        _f: &mut FdState,
        _buf: &[u8],
        _ts: u64,
    ) -> Result<usize, VfsError> {
        Err(VfsError::Unsupported)
    }

    fn mkdir(&mut self, _dev: &mut RawBlockDevice, _path: &str, _ts: u64) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn unlink(&mut self, _dev: &mut RawBlockDevice, _path: &str, _ts: u64) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn rename(
        &mut self,
        _dev: &mut RawBlockDevice,
        _old: &str,
        _new: &str,
        _ts: u64,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn truncate(
        &mut self,
        _dev: &mut RawBlockDevice,
        _path: &str,
        _size: u64,
        _ts: u64,
    ) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    fn sync(&mut self, _dev: &mut RawBlockDevice) -> Result<(), VfsError> {
        Err(VfsError::Unsupported)
    }

    /// Record a point-in-time marker; returns its handle (Helix: the snapshot
    /// LSN, usable for `O_AT_LSN` reads).
    fn snapshot(
        &mut self,
        _dev: &mut RawBlockDevice,
        _name: &str,
        _ts: u64,
    ) -> Result<u64, VfsError> {
        Err(VfsError::Unsupported)
    }

    /// History of `path`, oldest-first, as `(lsn, timestamp_ns, op)` where `op`
    /// is the backend's log-op discriminant (mirrors `FileVersion`).
    fn versions(
        &mut self,
        _dev: &mut RawBlockDevice,
        _path: &str,
    ) -> Result<Vec<(u64, u64, u32)>, VfsError> {
        Err(VfsError::Unsupported)
    }
}
