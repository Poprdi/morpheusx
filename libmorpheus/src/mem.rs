//! Memory management — mmap, munmap, alloc, free.

use crate::is_error;
use crate::raw::*;

/// Allocate physical pages from the kernel memory registry.
///
/// Returns the physical base address on success.
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

/// Map pages into the calling process's virtual address space.
///
/// Allocates `pages` physical pages and maps them contiguously into the
/// process's user virtual address space.  Returns the virtual address
/// of the first page.
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
