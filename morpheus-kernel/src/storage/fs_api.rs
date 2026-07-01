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

/// What an fd points at; dispatch branches on this. Regular fds go to the VFS and
/// are NOT pollable (`EPOLL_CTL_ADD` → `EPERM`); the rest go to their backend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FdKind {
    Regular,
    Socket,
    Pipe,
    Epoll,
}

impl FdKind {
    pub fn is_pollable(self) -> bool {
        !matches!(self, FdKind::Regular)
    }
}

/// Per-fd descriptor state owned by `FdTable` (the POSIX descriptor half): the
/// per-fd `FD_CLOEXEC` flag plus a handle (`ofd`) to the shared open-file
/// description holding the byte `offset` + status flags. `ofd == 0` ⇒ inline
/// `offset`/`flags` are authoritative; once `dup`/`dup2`/`F_DUPFD`/`try_clone`
/// alias the fd they move into the OFD and reads/writes MUST route through
/// `FdTable::{offset,set_offset,add_offset,status_flags,..}` (the dup-copies-offset
/// fix). `cookie` is backend-private; `FdKind::Socket` packs its handle in the low
/// 8 bytes (`socket_cookie`).
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
    pub kind: FdKind,
    /// `FD_CLOEXEC`: dropped by `spawn` (file-actions) before the child runs.
    pub cloexec: bool,
    /// 1-based handle into the shared OFD slab; 0 = private inline description.
    pub ofd: u32,
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
            kind: FdKind::Regular,
            cloexec: false,
            ofd: 0,
            in_use: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.in_use
    }

    pub fn is_socket(&self) -> bool {
        matches!(self.kind, FdKind::Socket)
    }

    pub fn is_pollable(&self) -> bool {
        self.kind.is_pollable()
    }

    /// Socket backend handle, packed in the first 8 cookie bytes (native endian).
    pub fn socket_cookie(&self) -> u64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.cookie[..8]);
        u64::from_ne_bytes(b)
    }

    pub fn set_socket_cookie(&mut self, handle: u64) {
        self.cookie[..8].copy_from_slice(&handle.to_ne_bytes());
    }

    pub fn path_str(&self) -> &str {
        let len = (self.path_len as usize).min(self.path.len());
        core::str::from_utf8(&self.path[..len]).unwrap_or("")
    }
}

/// Shared open-file-description (OFD) slab. `dup`/`dup2`/`F_DUPFD`/`try_clone` and
/// `fork`-style fd-table clones alias ONE refcounted OFD so they share a cursor +
/// status flags (POSIX, and the dup-copies-offset fix); last `decref` frees it.
/// One spinlock guards the slab — never nested under `PROCESS_TABLE_LOCK`.
mod ofd {
    use crate::sync::SpinLock;

    pub const MAX_OFD: usize = 1024;

    #[derive(Clone, Copy)]
    struct Ofd {
        offset: u64,
        status_flags: u32,
        refcount: u32,
        in_use: bool,
    }

    impl Ofd {
        const fn empty() -> Self {
            Self {
                offset: 0,
                status_flags: 0,
                refcount: 0,
                in_use: false,
            }
        }
    }

    static TABLE: SpinLock<[Ofd; MAX_OFD]> = SpinLock::new([Ofd::empty(); MAX_OFD]);

    #[inline]
    fn idx(handle: u32) -> Option<usize> {
        if handle == 0 {
            None
        } else {
            let i = (handle - 1) as usize;
            if i < MAX_OFD {
                Some(i)
            } else {
                None
            }
        }
    }

    /// Allocate a fresh OFD seeded from an inline description, refcount 1. Returns
    /// the 1-based handle, or 0 if the slab is full (caller keeps it inline).
    pub fn alloc(offset: u64, status_flags: u32) -> u32 {
        let mut t = TABLE.lock();
        for (i, slot) in t.iter_mut().enumerate() {
            if !slot.in_use {
                *slot = Ofd {
                    offset,
                    status_flags,
                    refcount: 1,
                    in_use: true,
                };
                return i as u32 + 1;
            }
        }
        0
    }

    pub fn incref(handle: u32) {
        if let Some(i) = idx(handle) {
            let mut t = TABLE.lock();
            if t[i].in_use {
                t[i].refcount = t[i].refcount.saturating_add(1);
            }
        }
    }

    pub fn decref(handle: u32) {
        if let Some(i) = idx(handle) {
            let mut t = TABLE.lock();
            if t[i].in_use {
                t[i].refcount = t[i].refcount.saturating_sub(1);
                if t[i].refcount == 0 {
                    t[i] = Ofd::empty();
                }
            }
        }
    }

    pub fn offset(handle: u32) -> u64 {
        idx(handle).map_or(0, |i| TABLE.lock()[i].offset)
    }

    pub fn set_offset(handle: u32, v: u64) {
        if let Some(i) = idx(handle) {
            TABLE.lock()[i].offset = v;
        }
    }

    pub fn add_offset(handle: u32, n: u64) -> u64 {
        match idx(handle) {
            Some(i) => {
                let mut t = TABLE.lock();
                t[i].offset = t[i].offset.saturating_add(n);
                t[i].offset
            },
            None => 0,
        }
    }

    pub fn flags(handle: u32) -> u32 {
        idx(handle).map_or(0, |i| TABLE.lock()[i].status_flags)
    }

    pub fn set_flags(handle: u32, v: u32) {
        if let Some(i) = idx(handle) {
            TABLE.lock()[i].status_flags = v;
        }
    }
}

/// Per-process fd table. Bounded (DoS limit, not an array-as-capacity decision)
/// and **not `Copy`** — duplicating one across processes must be explicit, so the
/// per-mount `open_fds` refcount stays accurate. Const-constructible for the
/// `PROCESS_TABLE` static.
pub const FD_TABLE_LEN: usize = 64;

pub struct FdTable {
    slots: [FdState; FD_TABLE_LEN],
}

/// Child SHARES every inherited OFD with the parent (POSIX: forked fds share the
/// parent's cursors), so each live shared slot bumps the OFD refcount.
impl Clone for FdTable {
    fn clone(&self) -> Self {
        for s in self.slots.iter() {
            if s.in_use && s.ofd != 0 {
                ofd::incref(s.ofd);
            }
        }
        Self { slots: self.slots }
    }
}

/// Releasing a table releases its share of every OFD it still holds — the last
/// reference frees the description.
impl Drop for FdTable {
    fn drop(&mut self) {
        for s in self.slots.iter() {
            if s.in_use && s.ofd != 0 {
                ofd::decref(s.ofd);
            }
        }
    }
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
        let ofd = slot.ofd;
        *slot = FdState::empty();
        if ofd != 0 {
            ofd::decref(ofd);
        }
        Some(mount_id)
    }

    /// Live `(fd, &FdState)` pairs — used by reap to close everything.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &FdState)> {
        self.slots.iter().enumerate().filter(|(_, s)| s.in_use)
    }

    // Read/write/seek MUST go through these, not raw `FdState.offset`/`.flags`: for a
    // shared (dup'd) fd the cursor lives in the OFD and the inline fields are stale.
    // Private fds (`ofd == 0`) fall back to inline, so the unshared path is unchanged.
    pub fn offset(&self, fd: usize) -> Option<u64> {
        let s = self.get(fd)?;
        Some(if s.ofd != 0 {
            ofd::offset(s.ofd)
        } else {
            s.offset
        })
    }

    pub fn set_offset(&mut self, fd: usize, v: u64) -> bool {
        match self.get_mut(fd) {
            Some(s) => {
                if s.ofd != 0 {
                    ofd::set_offset(s.ofd, v);
                } else {
                    s.offset = v;
                }
                true
            },
            None => false,
        }
    }

    /// Advance the cursor by `n`, returning the new offset (saturating).
    pub fn add_offset(&mut self, fd: usize, n: u64) -> Option<u64> {
        let s = self.get_mut(fd)?;
        if s.ofd != 0 {
            Some(ofd::add_offset(s.ofd, n))
        } else {
            s.offset = s.offset.saturating_add(n);
            Some(s.offset)
        }
    }

    /// OFD status flags (`O_NONBLOCK`/`O_APPEND`/access mode) — backs `F_GETFL`.
    pub fn status_flags(&self, fd: usize) -> Option<u32> {
        let s = self.get(fd)?;
        Some(if s.ofd != 0 {
            ofd::flags(s.ofd)
        } else {
            s.flags
        })
    }

    /// Set OFD status flags — backs `F_SETFL` / `FIONBIO`. Shared across every fd
    /// aliasing this description.
    pub fn set_status_flags(&mut self, fd: usize, v: u32) -> bool {
        match self.get_mut(fd) {
            Some(s) => {
                if s.ofd != 0 {
                    ofd::set_flags(s.ofd, v);
                } else {
                    s.flags = v;
                }
                true
            },
            None => false,
        }
    }

    pub fn get_cloexec(&self, fd: usize) -> Option<bool> {
        self.get(fd).map(|s| s.cloexec)
    }

    pub fn set_cloexec(&mut self, fd: usize, on: bool) -> bool {
        match self.get_mut(fd) {
            Some(s) => {
                s.cloexec = on;
                true
            },
            None => false,
        }
    }

    /// Drop every `FD_CLOEXEC` fd — the spawn-time step before file-actions replay.
    pub fn close_cloexec(&mut self) {
        for fd in 0..FD_TABLE_LEN {
            if self.get(fd).map(|s| s.cloexec).unwrap_or(false) {
                self.free(fd);
            }
        }
    }

    /// Promote `fd`'s inline description to a shared OFD (idempotent), returning
    /// its handle (0 if the slab is full — `fd` stays private).
    fn ensure_ofd(&mut self, fd: usize) -> u32 {
        let s = match self.get_mut(fd) {
            Some(s) => s,
            None => return 0,
        };
        if s.ofd != 0 {
            return s.ofd;
        }
        let h = ofd::alloc(s.offset, s.flags);
        if h != 0 {
            s.ofd = h;
        }
        h
    }

    /// `dup`: lowest free fd ≥ 3 aliasing `old`'s OFD (shared cursor). The new fd
    /// starts with `FD_CLOEXEC` clear (POSIX `dup`).
    pub fn dup(&mut self, old: usize) -> Result<usize, VfsError> {
        self.dup_from(old, 3, false)
    }

    /// `dup2`: make `new` alias `old`'s OFD, closing whatever `new` held. `new`
    /// may target stdio slots (0/1/2) for redirection. `FD_CLOEXEC` is cleared on
    /// `new` (POSIX). Returns `new`.
    pub fn dup2(&mut self, old: usize, new: usize) -> Result<usize, VfsError> {
        if new >= FD_TABLE_LEN {
            return Err(VfsError::BadFd);
        }
        if self.get(old).is_none() {
            return Err(VfsError::BadFd);
        }
        if old == new {
            return Ok(new);
        }
        let h = self.ensure_ofd(old);
        let mut src = *self.get(old).unwrap();
        if self.get(new).is_some() {
            self.free(new);
        }
        if h != 0 {
            ofd::incref(h);
        }
        src.ofd = h;
        src.cloexec = false;
        self.set(new, src);
        Ok(new)
    }

    /// `fcntl(F_DUPFD / F_DUPFD_CLOEXEC)` and `try_clone`: lowest free fd ≥
    /// `min_fd` (clamped to 3) aliasing `old`'s OFD, with `FD_CLOEXEC` = `cloexec`.
    pub fn dup_from(
        &mut self,
        old: usize,
        min_fd: usize,
        cloexec: bool,
    ) -> Result<usize, VfsError> {
        if self.get(old).is_none() {
            return Err(VfsError::BadFd);
        }
        let start = min_fd.max(3);
        let new = (start..FD_TABLE_LEN)
            .find(|&i| self.get(i).is_none())
            .ok_or(VfsError::TooManyOpen)?;
        let h = self.ensure_ofd(old);
        let mut src = *self.get(old).unwrap();
        if h != 0 {
            ofd::incref(h);
        }
        src.ofd = h;
        src.cloexec = cloexec;
        self.set(new, src);
        Ok(new)
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
