//! Persistent key-value storage and binary introspection.
//!
//! # Key-Value Store
//!
//! The persistence subsystem provides a simple key-value store backed by
//! HelixFS (files stored under `/persist/<key>`).  Keys must be non-empty,
//! at most 255 bytes, and may not contain `/` or `\0`.  Values can be up
//! to 4 MiB.
//!
//! ```ignore
//! use libmorpheus::persist;
//!
//! // Store a value
//! persist::put("my_app.cfg", b"theme=dark\nlang=en\n").unwrap();
//!
//! // Read it back
//! let mut buf = [0u8; 256];
//! let n = persist::get("my_app.cfg", &mut buf).unwrap();
//!
//! // Enumerate keys
//! let mut listing = [0u8; 4096];
//! let count = persist::list(&mut listing, 0).unwrap();
//!
//! // Delete
//! persist::del("my_app.cfg").unwrap();
//! ```
//!
//! # Binary Introspection
//!
//! `pe_info` parses PE32+ and ELF64 headers from a file on the VFS and
//! returns architecture, entry point, section count, etc.

use crate::raw::*;
use crate::{is_error, EINVAL};

// ═══════════════════════════════════════════════════════════════════════════
// FFI-compatible structs — must match hwinit/src/syscall/handler.rs exactly
// ═══════════════════════════════════════════════════════════════════════════

/// Persistence backend status and usage statistics.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PersistInfo {
    /// Bitmask: bit 0 = HelixFS backend active.
    pub backend_flags: u32,
    pub _pad0: u32,
    /// Number of keys currently stored.
    pub num_keys: u64,
    /// Total bytes used by stored values.
    pub used_bytes: u64,
}

/// Binary format information returned by [`pe_info`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BinaryInfo {
    /// Format: 0 = unknown, 1 = ELF64, 2 = PE32+.
    pub format: u32,
    /// Architecture: 0 = unknown, 1 = x86_64, 2 = aarch64, 3 = arm.
    pub arch: u32,
    /// Entry point address (RVA for PE, virtual address for ELF).
    pub entry_point: u64,
    /// PE `ImageBase` or 0 for ELF.
    pub image_base: u64,
    /// Total image/file size in bytes.
    pub image_size: u64,
    /// Number of sections (PE) or program headers (ELF).
    pub num_sections: u32,
    pub _pad0: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// Error helper
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a raw syscall return into `Result`.
/// Non-error values are returned as `Ok(v)`.
#[inline]
fn to_result(ret: u64) -> Result<u64, u64> {
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Key-Value Persistence
// ═══════════════════════════════════════════════════════════════════════════

/// Store a named blob to persistent storage.
///
/// Overwrites any existing value for `key`.
/// Returns `Ok(())` on success or the raw kernel error code.
///
/// # Limits
/// - `key`: 1–255 bytes, no `/` or `\0`.
/// - `data`: at most 4 MiB.
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

/// Load a named blob from persistent storage into `buf`.
///
/// Returns `Ok(bytes_read)` — the number of bytes actually written to `buf`.
/// If `buf` is shorter than the stored value, only `buf.len()` bytes are
/// returned (no error).
///
/// Returns `Err(ENOENT)` if the key does not exist.
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

/// Query the size of a stored blob *without* reading its contents.
///
/// Returns `Ok(size_bytes)` or `Err(ENOENT)`.
pub fn get_size(key: &str) -> Result<u64, u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe {
        syscall4(
            SYS_PERSIST_GET,
            key.as_ptr() as u64,
            key.len() as u64,
            0, // null buf → return size
            0, // zero length
        )
    };
    to_result(ret)
}

/// Delete a key from persistent storage.
///
/// Returns `Ok(())` on success, `Err(ENOENT)` if not found.
pub fn del(key: &str) -> Result<(), u64> {
    if key.is_empty() || key.len() > 255 {
        return Err(EINVAL);
    }
    let ret = unsafe {
        syscall2(
            SYS_PERSIST_DEL,
            key.as_ptr() as u64,
            key.len() as u64,
        )
    };
    to_result(ret).map(|_| ())
}

/// List keys in persistent storage.
///
/// Writes NUL-separated key names into `buf`.  `offset` skips that many
/// entries (for pagination when the buffer is too small).
///
/// Returns the number of keys written to the buffer.  Use
/// [`info`] to get the total key count.
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

/// Get persistence subsystem status and usage statistics.
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

// ═══════════════════════════════════════════════════════════════════════════
// Binary Introspection
// ═══════════════════════════════════════════════════════════════════════════

/// Parse PE32+ or ELF64 headers from a file and return binary metadata.
///
/// `path` must be a VFS path (e.g. `/bin/hello`).
///
/// Returns `Err(ENOENT)` if the file doesn't exist, `Err(EINVAL)` if too
/// small or not a recognised binary format (though `format == 0` is also
/// returned for unknown formats when the file *can* be read).
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

// ═══════════════════════════════════════════════════════════════════════════
// Convenience iterators
// ═══════════════════════════════════════════════════════════════════════════

/// Parse a NUL-separated key listing (as returned by [`list`]) into
/// individual key slices.
///
/// ```ignore
/// let mut buf = [0u8; 4096];
/// let count = persist::list(&mut buf, 0).unwrap();
/// for key in persist::parse_key_list(&buf) {
///     // key is &[u8]
/// }
/// ```
pub fn parse_key_list(buf: &[u8]) -> KeyListIter<'_> {
    KeyListIter { buf, pos: 0 }
}

/// Iterator over NUL-separated key names in a listing buffer.
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
        // Find next NUL or end of buffer.
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        if self.pos == start {
            return None; // empty name = end of data
        }
        let key = &self.buf[start..self.pos];
        // Skip the NUL separator.
        if self.pos < self.buf.len() {
            self.pos += 1;
        }
        Some(key)
    }
}
