//! Persistent KV store (backed by HelixFS) and PE/ELF binary introspection.

use crate::raw::*;
use crate::{is_error, EINVAL};

// FFI structs are canonical in morpheus-foundation — single source of truth.
pub use morpheus_foundation::types::{BinaryInfo, PersistInfo};

#[inline]
fn to_result(ret: u64) -> Result<u64, u64> {
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Store a blob under `key`, overwriting any existing value. Key must be
/// 1–255 bytes with no `/` or NUL; `data` is capped at 4 MiB.
pub fn put(key: &str, data: &[u8]) -> Result<(), u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe {
        syscall4(
            SYS_PERSIST_PUT,
            key.as_ptr() as u64,
            key.len() as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    to_result(ret).map(|_| ())
}

/// Read `key` into `buf`, truncating silently if `buf` is too small.
/// Returns bytes written, or `ENOENT`.
pub fn get(key: &str, buf: &mut [u8]) -> Result<usize, u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe {
        syscall4(
            SYS_PERSIST_GET,
            key.as_ptr() as u64,
            key.len() as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    to_result(ret).map(|v| v as usize)
}

/// Stat the value size without reading. Null buf is the size-only protocol.
pub fn get_size(key: &str) -> Result<u64, u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe { syscall4(SYS_PERSIST_GET, key.as_ptr() as u64, key.len() as u64, 0, 0) };
    to_result(ret)
}

pub fn del(key: &str) -> Result<(), u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe { syscall2(SYS_PERSIST_DEL, key.as_ptr() as u64, key.len() as u64) };
    to_result(ret).map(|_| ())
}

/// Write NUL-separated key names into `buf`, skipping the first `offset`
/// entries. Returns number of keys written; total count via [`info`].
pub fn list(buf: &mut [u8], offset: u64) -> Result<u64, u64> {
    let ret = unsafe {
        syscall3(
            SYS_PERSIST_LIST,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            offset,
        )
    };
    to_result(ret)
}

pub fn info() -> Result<PersistInfo, u64> {
    let mut pi = PersistInfo {
        backend_flags: 0,
        _pad0: 0,
        num_keys: 0,
        used_bytes: 0,
    };
    let ret = unsafe { syscall1(SYS_PERSIST_INFO, &mut pi as *mut PersistInfo as u64) };
    to_result(ret).map(|_| pi)
}

/// Parse PE32+/ELF64 headers from a VFS file. Unknown formats return
/// `format == 0` if the file is readable; `EINVAL` if it isn't.
pub fn pe_info(path: &str) -> Result<BinaryInfo, u64> {
    if path.is_empty() {
        return Err(EINVAL);
    }
    let mut bi = BinaryInfo {
        format: 0,
        arch: 0,
        entry_point: 0,
        image_base: 0,
        image_size: 0,
        num_sections: 0,
        _pad0: 0,
    };
    let ret = unsafe {
        syscall3(
            SYS_PE_INFO,
            path.as_ptr() as u64,
            path.len() as u64,
            &mut bi as *mut BinaryInfo as u64,
        )
    };
    to_result(ret).map(|_| bi)
}

/// Iterate over the NUL-separated key names produced by [`list`].
pub fn parse_key_list(buf: &[u8]) -> KeyListIter<'_> {
    KeyListIter { buf, pos: 0 }
}

pub struct KeyListIter<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for KeyListIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        if self.pos == start {
            return None; // trailing empty entry marks end
        }
        let key = &self.buf[start..self.pos];
        if self.pos < self.buf.len() {
            self.pos += 1;
        }
        Some(key)
    }
}
