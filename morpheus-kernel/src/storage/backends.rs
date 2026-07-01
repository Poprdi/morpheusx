//! FS adapters + the dispatch enum (spec §3 layer 3, §4). `MountedFs` is
//! match-dispatched (no `dyn`); adding an FS is a new variant + a new arm, and
//! the compiler flags every dispatch site that misses it. Each adapter wraps a
//! pure engine crate and maps its private error → `VfsError`.

use super::fs_api::{FdState, FsBackend, FsCapabilities, OpenFile, VfsError};
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use morpheus_block_types::{RawBlockDevice, RawIoError};
use morpheus_foundation::flags::{dirent_type, mode, open_flags};
use morpheus_foundation::storage::FD_COOKIE_LEN;
use morpheus_foundation::types::{DirEntry, FileStat};

/// Static-dispatch FS handle. One variant per backend; never a trait object.
// Boxing the large variant would put a vtable-free allocation in the I/O path,
// which the spec (§3) forbids; the size skew is intentional.
#[allow(clippy::large_enum_variant)]
pub enum MountedFs {
    Helix(HelixFs),
    Fat32(Fat32Fs),
}

impl MountedFs {
    pub fn capabilities(&self) -> FsCapabilities {
        match self {
            MountedFs::Helix(h) => h.capabilities(),
            MountedFs::Fat32(f) => f.capabilities(),
        }
    }
    pub fn open(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        flags: u32,
        ts: u64,
    ) -> Result<OpenFile, VfsError> {
        match self {
            MountedFs::Helix(h) => h.open(dev, path, flags, ts),
            MountedFs::Fat32(f) => f.open(dev, path, flags, ts),
        }
    }
    pub fn read(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &FdState,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        match self {
            MountedFs::Helix(h) => h.read(dev, f, buf),
            MountedFs::Fat32(fs) => fs.read(dev, f, buf),
        }
    }
    pub fn stat(&mut self, dev: &mut RawBlockDevice, path: &str) -> Result<FileStat, VfsError> {
        match self {
            MountedFs::Helix(h) => h.stat(dev, path),
            MountedFs::Fat32(f) => f.stat(dev, path),
        }
    }
    pub fn readdir(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
    ) -> Result<Vec<DirEntry>, VfsError> {
        match self {
            MountedFs::Helix(h) => h.readdir(dev, path),
            MountedFs::Fat32(f) => f.readdir(dev, path),
        }
    }
    pub fn close(&mut self, dev: &mut RawBlockDevice, f: &FdState) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.close(dev, f),
            MountedFs::Fat32(fs) => fs.close(dev, f),
        }
    }
    pub fn write(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &mut FdState,
        buf: &[u8],
        ts: u64,
    ) -> Result<usize, VfsError> {
        match self {
            MountedFs::Helix(h) => h.write(dev, f, buf, ts),
            MountedFs::Fat32(fs) => fs.write(dev, f, buf, ts),
        }
    }
    pub fn mkdir(&mut self, dev: &mut RawBlockDevice, path: &str, ts: u64) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.mkdir(dev, path, ts),
            MountedFs::Fat32(f) => f.mkdir(dev, path, ts),
        }
    }
    pub fn unlink(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        ts: u64,
    ) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.unlink(dev, path, ts),
            MountedFs::Fat32(f) => f.unlink(dev, path, ts),
        }
    }
    pub fn rename(
        &mut self,
        dev: &mut RawBlockDevice,
        old: &str,
        new: &str,
        ts: u64,
    ) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.rename(dev, old, new, ts),
            MountedFs::Fat32(f) => f.rename(dev, old, new, ts),
        }
    }
    pub fn truncate(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        size: u64,
        ts: u64,
    ) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.truncate(dev, path, size, ts),
            MountedFs::Fat32(f) => f.truncate(dev, path, size, ts),
        }
    }
    pub fn sync(&mut self, dev: &mut RawBlockDevice) -> Result<(), VfsError> {
        match self {
            MountedFs::Helix(h) => h.sync(dev),
            MountedFs::Fat32(f) => f.sync(dev),
        }
    }
    pub fn snapshot(
        &mut self,
        dev: &mut RawBlockDevice,
        name: &str,
        ts: u64,
    ) -> Result<u64, VfsError> {
        match self {
            MountedFs::Helix(h) => h.snapshot(dev, name, ts),
            MountedFs::Fat32(f) => f.snapshot(dev, name, ts),
        }
    }
    pub fn versions(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
    ) -> Result<Vec<(u64, u64, u32)>, VfsError> {
        match self {
            MountedFs::Helix(h) => h.versions(dev, path),
            MountedFs::Fat32(f) => f.versions(dev, path),
        }
    }
}

/// Public so the mount path can map a HelixFS engine `mount`/`format` error
/// (which happens outside the adapter) through the same table.
pub fn helix_err_pub(e: morpheus_helix::HelixError) -> VfsError {
    helix_err(e)
}

fn helix_err(e: morpheus_helix::HelixError) -> VfsError {
    use morpheus_helix::HelixError::*;
    match e {
        NotFound | MountNotFound | NoEntries => VfsError::NotFound,
        AlreadyExists => VfsError::Exists,
        IsADirectory => VfsError::IsDir,
        NotADirectory => VfsError::NotDir,
        DirectoryNotEmpty => VfsError::NotEmpty,
        InvalidFd => VfsError::BadFd,
        TooManyOpenFiles => VfsError::TooManyOpen,
        ReadOnly => VfsError::ReadOnly,
        NoSpace | LogFull | FileTooLarge => VfsError::NoSpace,
        PermissionDenied => VfsError::Perm,
        PathTooLong => VfsError::NameTooLong,
        InvalidOffset | PathInvalid | InvalidBlockSize | FormatTooSmall => VfsError::Inval,
        NotSupported => VfsError::Unsupported,
        IoReadFailed | IoWriteFailed | IoFlushFailed => VfsError::Io,
        _ => VfsError::Io,
    }
}

pub struct HelixFs {
    engine: morpheus_helix::HelixFs,
    read_only: bool,
}

impl HelixFs {
    pub fn new(engine: morpheus_helix::HelixFs, read_only: bool) -> Self {
        Self { engine, read_only }
    }
}

/// Pack/unpack the Helix per-fd cookie: the index key in the low 8 bytes.
fn helix_cookie_set(key: u64) -> [u8; FD_COOKIE_LEN] {
    let mut c = [0u8; FD_COOKIE_LEN];
    c[..8].copy_from_slice(&key.to_le_bytes());
    c
}

impl FsBackend for HelixFs {
    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            writable: !self.read_only,
            resizable: !self.read_only,
            snapshots: !self.read_only,
            versions: true,
        }
    }

    fn open(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        flags: u32,
        ts: u64,
    ) -> Result<OpenFile, VfsError> {
        if self.read_only && flags & (open_flags::O_WRITE | open_flags::O_CREATE) != 0 {
            return Err(VfsError::ReadOnly);
        }
        let key = self.engine.open(dev, path, flags, ts).map_err(helix_err)?;
        let is_dir = self
            .engine
            .stat(path)
            .map(|st| st.mode & mode::S_IFMT == mode::S_IFDIR)
            .unwrap_or(false);
        Ok(OpenFile {
            cookie: helix_cookie_set(key),
            is_dir,
        })
    }

    fn read(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &FdState,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        // Engine reads whole files; the fd offset slices here.
        let data = self.engine.read(dev, f.path_str()).map_err(helix_err)?;
        let off = f.offset as usize;
        if off >= data.len() {
            return Ok(0);
        }
        let n = buf.len().min(data.len() - off);
        buf[..n].copy_from_slice(&data[off..off + n]);
        Ok(n)
    }

    fn stat(&mut self, _dev: &mut RawBlockDevice, path: &str) -> Result<FileStat, VfsError> {
        self.engine.stat(path).map_err(helix_err)
    }

    fn readdir(
        &mut self,
        _dev: &mut RawBlockDevice,
        path: &str,
    ) -> Result<Vec<DirEntry>, VfsError> {
        self.engine.readdir(path).map_err(helix_err)
    }

    fn write(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &mut FdState,
        buf: &[u8],
        ts: u64,
    ) -> Result<usize, VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        // Read-modify-write: splice `buf` in at the fd offset (the engine is
        // whole-file). A missing file means "start empty" (fresh O_CREATE before
        // its first flush); any other read error must propagate, or a transient
        // I/O fault would silently clobber existing data with the spliced buffer.
        let mut data = match self.engine.read(dev, f.path_str()) {
            Ok(d) => d,
            Err(morpheus_helix::HelixError::NotFound) => Vec::new(),
            Err(e) => return Err(helix_err(e)),
        };
        let off = f.offset as usize;
        let end = off.checked_add(buf.len()).ok_or(VfsError::Inval)?;
        if end > data.len() {
            data.resize(end, 0u8);
        }
        data[off..end].copy_from_slice(buf);
        self.engine
            .write(dev, f.path_str(), &data, ts)
            .map_err(helix_err)?;
        f.offset = end as u64;
        Ok(buf.len())
    }

    fn mkdir(&mut self, dev: &mut RawBlockDevice, path: &str, ts: u64) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        self.engine.mkdir(dev, path, ts).map_err(helix_err)
    }

    fn unlink(&mut self, dev: &mut RawBlockDevice, path: &str, ts: u64) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        self.engine.unlink(dev, path, ts).map_err(helix_err)
    }

    fn rename(
        &mut self,
        dev: &mut RawBlockDevice,
        old: &str,
        new: &str,
        ts: u64,
    ) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        self.engine.rename(dev, old, new, ts).map_err(helix_err)
    }

    fn truncate(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        size: u64,
        ts: u64,
    ) -> Result<(), VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        self.engine.truncate(dev, path, size, ts).map_err(helix_err)
    }

    fn sync(&mut self, dev: &mut RawBlockDevice) -> Result<(), VfsError> {
        if self.read_only {
            return Ok(());
        }
        self.engine.sync(dev).map_err(helix_err)
    }

    fn snapshot(&mut self, dev: &mut RawBlockDevice, name: &str, ts: u64) -> Result<u64, VfsError> {
        if self.read_only {
            return Err(VfsError::ReadOnly);
        }
        self.engine.snapshot(dev, name, ts).map_err(helix_err)
    }

    fn versions(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
    ) -> Result<Vec<(u64, u64, u32)>, VfsError> {
        let recs = self.engine.versions(dev, path).map_err(helix_err)?;
        Ok(recs
            .into_iter()
            .map(|(lsn, ts_ns, op)| (lsn, ts_ns, op as u32))
            .collect())
    }
}

// FAT32 adapter (read-only). fat32's engine owns its `B: BlockIo` with a private
// field. `DevPtr` bridges registry ownership: the adapter holds a boxed
// `AtomicPtr` slot (stable address); before each op it stores the borrowed device
// into the slot; the engine's I/O reads through it. Device never leaves the
// registry; engine never sees it outside one op's borrow.

use core::sync::atomic::{AtomicPtr, Ordering};

struct DevPtr {
    /// Points at the adapter's `Box<AtomicPtr<RawBlockDevice>>`; stable for the
    /// adapter's life. Repointed indirectly via `bind`.
    slot: *const AtomicPtr<RawBlockDevice>,
}

// SAFETY: the slot is only loaded while the adapter has just stored a live,
// uniquely-borrowed RawBlockDevice under STORAGE_LOCK; never raced across cores.
unsafe impl Send for DevPtr {}
unsafe impl Sync for DevPtr {}

impl DevPtr {
    #[inline]
    fn dev(&self) -> *mut RawBlockDevice {
        // SAFETY: `slot` points at the adapter's boxed AtomicPtr, alive for the
        // engine's lifetime.
        unsafe { (*self.slot).load(Ordering::Relaxed) }
    }
}

impl BlockIo for DevPtr {
    type Error = RawIoError;
    fn block_size(&self) -> BlockSize {
        // SAFETY: dev() is a live RawBlockDevice the adapter bound for this op.
        unsafe { (*self.dev()).block_size() }
    }
    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        // SAFETY: see block_size.
        unsafe { (*self.dev()).num_blocks() }
    }
    fn read_blocks(&mut self, start: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        // SAFETY: see block_size.
        unsafe { (*self.dev()).read_blocks(start, dst) }
    }
    fn write_blocks(&mut self, start: Lba, src: &[u8]) -> Result<(), Self::Error> {
        // SAFETY: see block_size.
        unsafe { (*self.dev()).write_blocks(start, src) }
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        // SAFETY: see block_size.
        unsafe { (*self.dev()).flush() }
    }
}

fn fat32_err(e: morpheus_fat32::error::Fat32Error) -> VfsError {
    use morpheus_fat32::error::Fat32Error::*;
    match e {
        NotFound => VfsError::NotFound,
        NotADirectory => VfsError::NotDir,
        IsADirectory => VfsError::IsDir,
        PathTooLong => VfsError::NameTooLong,
        PathInvalid | InvalidOffset | NotFat32 | BadGeometry | InvalidBlockSize => VfsError::Inval,
        ReadOnly => VfsError::ReadOnly,
        IoRead | IoWrite | ChainCorrupt => VfsError::Io,
    }
}

pub struct Fat32Fs {
    engine: morpheus_fat32::Fat32Fs<DevPtr>,
    /// Stable-address device slot the engine's `DevPtr` reads through. Boxed so
    /// the engine's stored pointer to it stays valid across moves of the adapter.
    slot: alloc::boxed::Box<AtomicPtr<RawBlockDevice>>,
}

impl Fat32Fs {
    /// Mount a FAT32 volume at `lba_start`, binding `dev` into the shared slot.
    pub fn mount(dev: &mut RawBlockDevice, lba_start: u64) -> Result<Self, VfsError> {
        let slot = alloc::boxed::Box::new(AtomicPtr::new(dev as *mut RawBlockDevice));
        let bridge = DevPtr {
            slot: slot.as_ref() as *const AtomicPtr<RawBlockDevice>,
        };
        let engine = morpheus_fat32::Fat32Fs::open(bridge, lba_start).map_err(fat32_err)?;
        Ok(Self { engine, slot })
    }

    /// Point the shared slot at the current op's device.
    fn bind(&mut self, dev: &mut RawBlockDevice) {
        self.slot
            .store(dev as *mut RawBlockDevice, Ordering::Relaxed);
    }
}

/// FAT32 cookie layout in the fd cookie blob: [start_cluster:u32][cursor:u64][size:u64].
fn fat_cookie_set(c: &morpheus_fat32::Fat32Cookie) -> [u8; FD_COOKIE_LEN] {
    let mut out = [0u8; FD_COOKIE_LEN];
    out[0..4].copy_from_slice(&c.start_cluster.to_le_bytes());
    out[4..12].copy_from_slice(&c.cursor.to_le_bytes());
    out[12..20].copy_from_slice(&c.size.to_le_bytes());
    out
}

fn fat_cookie_get(c: &[u8; FD_COOKIE_LEN]) -> morpheus_fat32::Fat32Cookie {
    let mut sc = [0u8; 4];
    sc.copy_from_slice(&c[0..4]);
    let mut cur = [0u8; 8];
    cur.copy_from_slice(&c[4..12]);
    let mut sz = [0u8; 8];
    sz.copy_from_slice(&c[12..20]);
    morpheus_fat32::Fat32Cookie {
        start_cluster: u32::from_le_bytes(sc),
        cursor: u64::from_le_bytes(cur),
        size: u64::from_le_bytes(sz),
    }
}

impl FsBackend for Fat32Fs {
    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities::default() // read-only: everything false
    }

    fn open(
        &mut self,
        dev: &mut RawBlockDevice,
        path: &str,
        flags: u32,
        _ts: u64,
    ) -> Result<OpenFile, VfsError> {
        if flags & (open_flags::O_WRITE | open_flags::O_CREATE | open_flags::O_TRUNC) != 0 {
            return Err(VfsError::ReadOnly);
        }
        self.bind(dev);
        // A directory open is allowed for readdir; reads against it return IsDir.
        let st = self.engine.stat(path).map_err(fat32_err)?;
        let is_dir = matches!(st.file_type, morpheus_fat32::types::FileType::Directory);
        if is_dir {
            return Ok(OpenFile {
                cookie: [0u8; FD_COOKIE_LEN],
                is_dir: true,
            });
        }
        let cookie = self.engine.open_file(path).map_err(fat32_err)?;
        Ok(OpenFile {
            cookie: fat_cookie_set(&cookie),
            is_dir: false,
        })
    }

    fn read(
        &mut self,
        dev: &mut RawBlockDevice,
        f: &FdState,
        buf: &mut [u8],
    ) -> Result<usize, VfsError> {
        self.bind(dev);
        let mut cookie = fat_cookie_get(&f.cookie);
        // Honor the fd's persisted offset rather than the cookie's own cursor so
        // seeks work; the engine advances `cursor` from this base.
        cookie.cursor = f.offset;
        self.engine.read(&mut cookie, buf).map_err(fat32_err)
    }

    fn stat(&mut self, dev: &mut RawBlockDevice, path: &str) -> Result<FileStat, VfsError> {
        self.bind(dev);
        let st = self.engine.stat(path).map_err(fat32_err)?;
        Ok(fat_stat_to_abi(&st))
    }

    fn readdir(&mut self, dev: &mut RawBlockDevice, path: &str) -> Result<Vec<DirEntry>, VfsError> {
        self.bind(dev);
        let ents = self.engine.readdir(path).map_err(fat32_err)?;
        Ok(ents.iter().map(fat_dirent_to_abi).collect())
    }
}

fn fat_stat_to_abi(st: &morpheus_fat32::types::FileStat) -> FileStat {
    let is_dir = matches!(st.file_type, morpheus_fat32::types::FileType::Directory);
    FileStat {
        key: st.start_cluster as u64,
        size: st.size,
        mode: if is_dir { mode::S_IFDIR } else { mode::S_IFREG },
        version_count: 1,
        ..FileStat::default()
    }
}

fn fat_dirent_to_abi(e: &morpheus_fat32::types::DirEntry) -> DirEntry {
    let mut name = [0u8; 256];
    let bytes = e.name.as_bytes();
    let n = bytes.len().min(256);
    name[..n].copy_from_slice(&bytes[..n]);
    let is_dir = matches!(e.file_type, morpheus_fat32::types::FileType::Directory);
    DirEntry {
        name,
        name_len: n as u16,
        d_type: if is_dir {
            dirent_type::DT_DIR
        } else {
            dirent_type::DT_REG
        },
        size: e.size,
        version_count: 1,
        ..DirEntry::zeroed()
    }
}
