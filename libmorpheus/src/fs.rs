//! Filesystem operations — high-level wrappers around FS syscalls.

use crate::raw::*;
use crate::is_error;

pub const O_READ:   u32 = 0x01;
pub const O_WRITE:  u32 = 0x02;
pub const O_CREATE: u32 = 0x04;
pub const O_TRUNC:  u32 = 0x10;
pub const O_APPEND: u32 = 0x20;

pub const SEEK_SET: u64 = 0;
pub const SEEK_CUR: u64 = 1;
pub const SEEK_END: u64 = 2;

/// Open a file. Returns fd or negative error.
pub fn open(path: &str, flags: u32) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_OPEN,
            path.as_ptr() as u64,
            path.len() as u64,
            flags as u64,
        )
    };
    if is_error(ret) { Err(ret) } else { Ok(ret as usize) }
}

/// Read from fd into buf. Returns bytes read.
pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
    };
    if is_error(ret) { Err(ret) } else { Ok(ret as usize) }
}

/// Write buf to fd. Returns bytes written.
pub fn write(fd: usize, data: &[u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(SYS_WRITE, fd as u64, data.as_ptr() as u64, data.len() as u64)
    };
    if is_error(ret) { Err(ret) } else { Ok(ret as usize) }
}

pub fn close(fd: usize) -> Result<(), u64> {
    let ret = unsafe { syscall1(SYS_CLOSE, fd as u64) };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn seek(fd: usize, offset: i64, whence: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_SEEK, fd as u64, offset as u64, whence) };
    if is_error(ret) { Err(ret) } else { Ok(ret) }
}

pub fn mkdir(path: &str) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_MKDIR, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn unlink(path: &str) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_UNLINK, path.as_ptr() as u64, path.len() as u64) };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn rename(old: &str, new: &str) -> Result<(), u64> {
    let ret = unsafe {
        syscall4(
            SYS_RENAME,
            old.as_ptr() as u64, old.len() as u64,
            new.as_ptr() as u64, new.len() as u64,
        )
    };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn stat(path: &str, buf: &mut [u8]) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(SYS_STAT, path.as_ptr() as u64, path.len() as u64, buf.as_mut_ptr() as u64)
    };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}

pub fn sync() -> Result<(), u64> {
    let ret = unsafe { syscall0(SYS_SYNC) };
    if is_error(ret) { Err(ret) } else { Ok(()) }
}
