//! Memory: mmap, munmap, shared memory, mprotect.

use crate::is_error;
use crate::raw::*;

/// Get physical pages from the kernel.
pub fn alloc_pages(pages: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_ALLOC, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Free previously allocated physical pages.
pub fn free_pages(phys_base: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_FREE, phys_base, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Map `pages` into our address space. kernel picks the VA.
pub fn mmap(pages: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall1(SYS_MMAP, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Unmap pages from the calling process's virtual address space.
pub fn munmap(vaddr: u64, pages: u64) -> Result<(), u64> {
    let ret = unsafe { syscall2(SYS_MUNMAP, vaddr, pages) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

// shared memory

pub const PROT_READ: u64 = 0; // always implied on x86-64
pub const PROT_WRITE: u64 = 1;
pub const PROT_EXEC: u64 = 2; // clears NX

/// Share physical pages with another process. they get a mapping, we keep ownership.
pub fn shm_grant(target_pid: u32, src_vaddr: u64, pages: u64, flags: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall4(SYS_SHM_GRANT, target_pid as u64, src_vaddr, pages, flags) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// mprotect

/// Flip page protection bits on an existing VMA.
pub fn mprotect(vaddr: u64, pages: u64, prot: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_MPROTECT, vaddr, pages, prot) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}
