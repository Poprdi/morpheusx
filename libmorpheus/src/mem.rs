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

// ═══════════════════════════════════════════════════════════════════════════
// SHARED MEMORY — SYS_SHM_GRANT (73)
// ═══════════════════════════════════════════════════════════════════════════

/// Protection flag: writable.
pub const PROT_READ: u64 = 0;

/// Protection flag: writable.
pub const PROT_WRITE: u64 = 1;

/// Protection flag: executable (clears NX on x86-64).
pub const PROT_EXEC: u64 = 2;

/// Grant shared physical pages to another process.
///
/// The caller must own the pages at `src_vaddr` (obtained via `mmap` or
/// `dma_alloc` → `map_phys`).  The kernel maps the same physical frames
/// into the target process's address space and returns the virtual address
/// in the target.
///
/// `flags` is a bitmask: `PROT_WRITE` (bit 0) and `PROT_EXEC` (bit 1).
/// `PROT_READ` is always implied (x86-64 has no read-only-without-present).
///
/// The target receives a non-owning mapping: `munmap` in the target will
/// unmap but NOT free the physical pages.  The granter retains ownership.
///
/// # Errors
///
/// - `EINVAL` — bad args, self-grant, page count mismatch
/// - `EPERM` — caller doesn't own the physical pages
/// - `ESRCH` — target process doesn't exist
/// - `ENOMEM` — target's VMA table or page tables are full
pub fn shm_grant(target_pid: u32, src_vaddr: u64, pages: u64, flags: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall4(SYS_SHM_GRANT, target_pid as u64, src_vaddr, pages, flags) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY PROTECTION — SYS_MPROTECT (74)
// ═══════════════════════════════════════════════════════════════════════════

/// Change the page protection flags on an existing mapping.
///
/// `vaddr` must be the exact start of a VMA (returned by `mmap` or
/// `shm_grant`), and `pages` must match the VMA's page count exactly.
///
/// `prot` is a bitmask: `PROT_WRITE` (bit 0) and `PROT_EXEC` (bit 1).
/// All other bits must be zero.  `PROT_READ` is always implied.
///
/// # Errors
///
/// - `EINVAL` — bad args, no matching VMA
/// - `EFAULT` — internal page table corruption
pub fn mprotect(vaddr: u64, pages: u64, prot: u64) -> Result<(), u64> {
    let ret = unsafe { syscall3(SYS_MPROTECT, vaddr, pages, prot) };
    if is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}
