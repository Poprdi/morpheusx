//! Memory: mmap, munmap, shared memory, mprotect.

use crate::is_error;
use crate::raw::*;

pub fn alloc_pages(pages: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_ALLOC, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn free_pages(phys_base: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_FREE, phys_base, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// PROT_*/MAP_* are canonical in morpheus-foundation — single source of truth.
pub use morpheus_foundation::flags::{
    MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, MAP_SHARED, PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE,
};

/// Raw mmap: anonymous, zero-filled, kernel-chosen VA, read+write. Returns the
/// VA or a `-errno` value (`is_error`); the shared entry every other mmap
/// wrapper routes through onto the `(pages, prot, flags, addr)` ABI.
#[inline]
pub fn mmap_raw(pages: u64) -> u64 {
    unsafe { syscall4(SYS_MMAP, pages, PROT_WRITE, MAP_ANONYMOUS | MAP_PRIVATE, 0) }
}

/// Kernel picks the VA; anonymous read+write.
pub fn mmap(pages: u64) -> Result<u64, u64> {
    let ret = mmap_raw(pages);
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Anonymous mapping with explicit protection; kernel picks the VA.
pub fn mmap_prot(pages: u64, prot: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall4(SYS_MMAP, pages, prot, MAP_ANONYMOUS | MAP_PRIVATE, 0) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Anonymous mapping pinned at `addr` (MAP_FIXED) with explicit protection.
pub fn mmap_fixed(addr: u64, pages: u64, prot: u64) -> Result<u64, u64> {
    let ret = unsafe {
        syscall4(
            SYS_MMAP,
            pages,
            prot,
            MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
            addr,
        )
    };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn munmap(vaddr: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_MUNMAP, vaddr, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Share physical pages with `target_pid`; we retain ownership.
pub fn shm_grant(target_pid: u32, src_vaddr: u64, pages: u64, flags: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall4(SYS_SHM_GRANT, target_pid as u64, src_vaddr, pages, flags) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

pub fn mprotect(vaddr: u64, pages: u64, prot: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_MPROTECT, vaddr, pages, prot) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}
