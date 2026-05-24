//! Filesystem syscalls and RAII wrappers (`File`, `OpenOptions`, `ReadDir`).

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{self, Error, ErrorKind};
use crate::io::{self, Read, Seek, SeekFrom, Write};
use crate::is_error;
use crate::raw::*;

pub const O_READ: u32 = 0x01;
pub const O_WRITE: u32 = 0x02;
pub const O_CREATE: u32 = 0x04;
pub const O_TRUNC: u32 = 0x10;
pub const O_APPEND: u32 = 0x20;

pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

pub fn open(path: &str, flags: u32) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_OPEN,
            path.as_ptr() as u64,
            path.len() as u64,
            flags as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_READ,
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

pub fn write(fd: usize, data: &[u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_WRITE,
            fd as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

pub fn close(fd: usize) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_CLOSE, fd as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn seek(fd: usize, offset: i64, whence: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_SEEK, fd as u64, offset as u64, whence) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn mkdir(path: &str) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_MKDIR, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn unlink(path: &str) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_UNLINK, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn rename(old: &str, new: &str) -> Result<(), u64> {
    let ret = unsafe {
        syscall4(
            SYS_RENAME,
            old.as_ptr() as u64,
            old.len() as u64,
            new.as_ptr() as u64,
            new.len() as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn stat(path: &str, buf: &mut [u8]) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_STAT,
            path.as_ptr() as u64,
            path.len() as u64,
            buf.as_mut_ptr() as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn sync() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_SYNC) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

pub fn dup(old_fd: usize) -> Result<usize, u64> {
    let ret = unsafe { syscall1(SYS_DUP, old_fd as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Writes the CWD into `buf`; returns bytes written.
pub fn getcwd(buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

pub fn chdir(path: &str) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_CHDIR, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Returns the number of entries written into `buf`.
pub fn readdir(path: &str, buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_READDIR,
            path.as_ptr() as u64,
            path.len() as u64,
            buf.as_mut_ptr() as u64,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// RAII fd; closed on drop. Implements `Read`, `Write`, `Seek`.
pub struct File {
    fd: usize,
}

impl File {
    pub fn open(path: &str) -> error::Result<Self> {
        let fd = open(path, O_READ).map_err(Error::from_raw)?;
        Ok(Self { fd })
    }

    /// Create or truncate for writing.
    pub fn create(path: &str) -> error::Result<Self> {
        let fd = open(path, O_WRITE | O_CREATE | O_TRUNC).map_err(Error::from_raw)?;
        Ok(Self { fd })
    }

    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// Caller asserts the fd is valid; will be closed on drop.
    pub fn from_raw_fd(fd: usize) -> Self {
        Self { fd }
    }

    /// Release ownership of the fd without closing.
    pub fn into_raw_fd(self) -> usize {
        let fd = self.fd;
        core::mem::forget(self);
        fd
    }

    pub fn fd(&self) -> usize {
        self.fd
    }

    pub fn metadata(&self) -> error::Result<Metadata> {
        // No fstat(fd); use `fs::metadata(path)` instead.
        Err(Error::new(ErrorKind::NotImplemented))
    }

    pub fn sync_all(&self) -> error::Result<()> {
        sync().map_err(Error::from_raw)
    }

    pub fn try_clone(&self) -> error::Result<Self> {
        let new_fd = dup(self.fd).map_err(Error::from_raw)?;
        Ok(Self { fd: new_fd })
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        read(self.fd, buf).map_err(Error::from_raw)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        write(self.fd, buf).map_err(Error::from_raw)
    }

    fn flush(&mut self) -> error::Result<()> {
        // HelixFS has no per-fd flush; use `sync()` for the global one.
        Ok(())
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> error::Result<u64> {
        let (offset, whence) = match pos {
            SeekFrom::Start(n) => (n as i64, SEEK_SET),
            SeekFrom::Current(n) => (n, SEEK_CUR),
            SeekFrom::End(n) => (n, SEEK_END),
        };
        seek(self.fd, offset, whence).map_err(Error::from_raw)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = close(self.fd);
    }
}

pub struct OpenOptions {
    read: bool,
    write: bool,
    create: bool,
    truncate: bool,
    append: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            create: false,
            truncate: false,
            append: false,
        }
    }

    pub fn read(&mut self, yes: bool) -> &mut Self {
        self.read = yes;
        self
    }
    pub fn write(&mut self, yes: bool) -> &mut Self {
        self.write = yes;
        self
    }
    pub fn create(&mut self, yes: bool) -> &mut Self {
        self.create = yes;
        self
    }
    pub fn truncate(&mut self, yes: bool) -> &mut Self {
        self.truncate = yes;
        self
    }
    pub fn append(&mut self, yes: bool) -> &mut Self {
        self.append = yes;
        self
    }

    pub fn open(&self, path: &str) -> error::Result<File> {
        let mut flags: u32 = 0;
        if self.read {
            flags |= O_READ;
        }
        if self.write {
            flags |= O_WRITE;
        }
        if self.create {
            flags |= O_CREATE;
        }
        if self.truncate {
            flags |= O_TRUNC;
        }
        if self.append {
            flags |= O_APPEND;
        }
        let fd = open(path, flags).map_err(Error::from_raw)?;
        Ok(File { fd })
    }
}

/// Mirrors `morpheus_helix::types::FileStat` — layout must match.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Metadata {
    pub key: u64,
    pub size: u64,
    pub is_dir: bool,
    /// TSC nanoseconds since boot.
    pub created_ns: u64,
    /// TSC nanoseconds since boot.
    pub modified_ns: u64,
    pub version_count: u32,
    pub lsn: u64,
    /// Creation LSN.
    pub first_lsn: u64,
    pub flags: u32,
}

impl Metadata {
    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    pub fn is_file(&self) -> bool {
        !self.is_dir
    }
}

pub fn metadata(path: &str) -> error::Result<Metadata> {
    let mut buf = [0u8; 128];
    stat(path, &mut buf).map_err(Error::from_raw)?;
    // Kernel writes `FileStat` raw; reinterpret.
    let ptr = buf.as_ptr() as *const Metadata;
    Ok(unsafe { core::ptr::read_unaligned(ptr) })
}

/// Mirrors `morpheus_helix::types::DirEntry` — layout must match.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DirEntry {
    name_buf: [u8; 256],
    name_len: u16,
    is_dir: bool,
    size: u64,
    modified_ns: u64,
    version_count: u32,
}

impl DirEntry {
    pub fn name(&self) -> &str {
        let len = (self.name_len as usize).min(self.name_buf.len());
        core::str::from_utf8(&self.name_buf[..len]).unwrap_or("")
    }

    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    pub fn is_file(&self) -> bool {
        !self.is_dir
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn modified_ns(&self) -> u64 {
        self.modified_ns
    }

    pub fn version_count(&self) -> u32 {
        self.version_count
    }
}

impl core::fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DirEntry")
            .field("name", &self.name())
            .field("is_dir", &self.is_dir)
            .field("size", &self.size)
            .finish()
    }
}

pub struct ReadDir {
    entries: Vec<DirEntry>,
    pos: usize,
}

impl Iterator for ReadDir {
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.entries.len() {
            let entry = self.entries[self.pos];
            self.pos += 1;
            Some(entry)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.entries.len() - self.pos;
        (remaining, Some(remaining))
    }
}

pub fn read_dir(path: &str) -> error::Result<ReadDir> {
    let entry_size = core::mem::size_of::<DirEntry>();
    let max_entries = 256;
    let mut buf = vec![0u8; entry_size * max_entries];

    let count = readdir(path, &mut buf).map_err(Error::from_raw)?;

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * entry_size;
        let ptr = buf[offset..].as_ptr() as *const DirEntry;
        entries.push(unsafe { core::ptr::read_unaligned(ptr) });
    }

    Ok(ReadDir { entries, pos: 0 })
}

pub fn read_to_vec(path: &str) -> error::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn read_to_string(path: &str) -> error::Result<String> {
    let mut f = File::open(path)?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    Ok(buf)
}

/// Truncate-or-create then write.
pub fn write_bytes(path: &str, data: &[u8]) -> error::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    Ok(())
}

pub fn create_dir(path: &str) -> error::Result<()> {
    mkdir(path).map_err(Error::from_raw)
}

pub fn remove_file(path: &str) -> error::Result<()> {
    unlink(path).map_err(Error::from_raw)
}

pub fn rename_path(old: &str, new: &str) -> error::Result<()> {
    rename(old, new).map_err(Error::from_raw)
}

pub fn copy(src: &str, dst: &str) -> error::Result<u64> {
    let mut reader = File::open(src)?;
    let mut writer = File::create(dst)?;
    io::copy(&mut reader, &mut writer)
}
