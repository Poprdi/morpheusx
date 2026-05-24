//! Compositor protocol: one process registers via [`compositor_set`]; thereafter
//! [`crate::hw::fb_map`] hands every other process a private offscreen surface.

use crate::raw::*;

/// Mirror of `hwinit::syscall::handler::SurfaceEntry` — layout must match exactly.
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

/// Register the caller as the compositor. Only one at a time.
pub fn compositor_set() -> Result<(), u64> {
    let r = unsafe { syscall0(SYS_COMPOSITOR_SET) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}

/// Enumerate surfaces. Empty `buf` returns total count for pre-sizing.
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

/// Map another process's surface into our address space until that process exits.
pub fn surface_map(pid: u32) -> Result<*mut u8, u64> {
    let r = unsafe { syscall1(SYS_WIN_SURFACE_MAP, pid as u64) };
    if r > 0xFFFF_FFFF_FFFF_FF00 {
        Err(r)
    } else {
        Ok(r as *mut u8)
    }
}

/// Route mouse delta to a process's per-process accumulator.
pub fn mouse_forward(pid: u32, dx: i16, dy: i16, buttons: u8) -> Result<(), u64> {
    let packed = (dx as u16 as u64) | ((dy as u16 as u64) << 16) | ((buttons as u64) << 32);
    let r = unsafe { syscall2(SYS_MOUSE_FORWARD, pid as u64, packed) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}

/// Push bytes into a target process's input ring; wakes it if blocked on `read(0)`.
/// Returns bytes written; may be short if the ring is full.
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

/// Called by the compositor after it composites a surface.
pub fn surface_dirty_clear(pid: u32) -> Result<(), u64> {
    let r = unsafe { syscall1(SYS_WIN_SURFACE_DIRTY_CLEAR, pid as u64) };
    if r == 0 {
        Ok(())
    } else {
        Err(r)
    }
}
