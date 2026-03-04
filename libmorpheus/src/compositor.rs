//! Compositor protocol — userspace API for per-process window surfaces.
//!
//! A single process registers as the compositor via [`compositor_set`].
//! After that, all other processes that call [`crate::hw::fb_map`] receive
//! private offscreen framebuffers.  The compositor reads those surfaces,
//! composites them onto the real framebuffer, and presents the result.

use crate::raw::*;

/// Surface descriptor returned by [`surface_list`].
///
/// Must match `hwinit::syscall::handler::SurfaceEntry` exactly.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SurfaceEntry {
    pub pid: u32,
    pub _pad: u32,
    pub phys_addr: u64,
    pub pages: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub dirty: u32,
    pub _pad2: u32,
}

/// Register the calling process as the window compositor.
///
/// Only one process can hold this role at a time.  Returns `Ok(())` on
/// success, `Err(code)` if another compositor is already registered.
pub fn compositor_set() -> Result<(), u64> {
    let r = unsafe { syscall0(SYS_COMPOSITOR_SET) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}

/// Enumerate all per-process framebuffer surfaces.
///
/// Fills `buf` with up to `buf.len()` entries and returns the number
/// of surfaces actually written.  If `buf` is empty, returns the total
/// count of active surfaces (for pre-sizing the buffer).
pub fn surface_list(buf: &mut [SurfaceEntry]) -> usize {
    let r = unsafe {
        syscall2(
            SYS_WIN_SURFACE_LIST,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    r as usize
}

/// Map another process's surface into our address space.
///
/// Returns a pointer to the surface pixel data.  The mapping is writable
/// (compositor may clear/init) but normally used read-only for compositing.
///
/// The returned pointer remains valid until the target process exits.
pub fn surface_map(pid: u32) -> Result<*mut u8, u64> {
    let r = unsafe { syscall1(SYS_WIN_SURFACE_MAP, pid as u64) };
    // Error codes are near u64::MAX (> 0xFFFF_FFFF_FFFF_FF00).
    if r > 0xFFFF_FFFF_FFFF_FF00 {
        Err(r)
    } else {
        Ok(r as *mut u8)
    }
}

/// Forward mouse input to a target process's per-process accumulator.
///
/// The compositor reads raw mouse data and routes it to the focused
/// window's process.
pub fn mouse_forward(pid: u32, dx: i16, dy: i16, buttons: u8) -> Result<(), u64> {
    let packed = (dx as u16 as u64) | ((dy as u16 as u64) << 16) | ((buttons as u64) << 32);
    let r = unsafe { syscall2(SYS_MOUSE_FORWARD, pid as u64, packed) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}

/// Forward keyboard bytes to a target process's per-process input buffer.
///
/// The compositor calls this after reading from global stdin and deciding
/// which child gets the input.  The kernel writes the bytes into the
/// target's `input_buf` ring buffer, where a subsequent `read(fd=0)` by
/// the child will find them.  Wakes the child if it was blocked.
///
/// Returns the number of bytes actually written (may be less than
/// `data.len()` if the target's buffer is full — maybe they should
/// read faster).
pub fn forward_input(pid: u32, data: &[u8]) -> Result<usize, u64> {
    if data.is_empty() {
        return Ok(0);
    }
    let r = unsafe {
        syscall3(
            SYS_FORWARD_INPUT,
            pid as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    if crate::is_error(r) {
        Err(r)
    } else {
        Ok(r as usize)
    }
}

/// Clear the dirty flag on a target process's surface.
///
/// Called by the compositor after it has read and composited the surface.
pub fn surface_dirty_clear(pid: u32) -> Result<(), u64> {
    let r = unsafe { syscall1(SYS_WIN_SURFACE_DIRTY_CLEAR, pid as u64) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}
