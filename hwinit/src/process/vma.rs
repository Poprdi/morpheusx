//! Per-process Virtual Memory Area (VMA) tracker.
//!
//! Records every user-space mapping created by SYS_MMAP / SYS_MAP_PHYS so
//! that SYS_MUNMAP can:
//!   1. Find the physical address backing a virtual range
//!   2. Unmap from the correct per-process page table (not the kernel's)
//!   3. Free the physical pages back to the buddy allocator
//!
//! # Design
//!
//! Fixed-size inline array — no heap, no linked lists, no abstractions
//! beyond what is strictly necessary.  Each entry records exactly one
//! contiguous mapping.  64 entries is generous for an exokernel where
//! user processes are expected to manage their own memory policies.
//!
//! # Invariants
//!
//! - `vaddr` is always page-aligned (4 KiB)
//! - `phys` is always page-aligned (4 KiB) — or 0 for foreign-phys mappings
//! - `pages > 0` for valid entries
//! - No two valid entries overlap in virtual address space
//! - `owns_phys == true`  → MUNMAP will free physical pages via MemoryRegistry
//! - `owns_phys == false` → MUNMAP unmaps PTEs only (MAP_PHYS / FB_MAP case)

/// Maximum number of VMA entries per process.
pub const MAX_VMAS: usize = 64;

/// A single virtual memory area descriptor.
#[derive(Clone, Copy)]
pub struct Vma {
    /// Virtual base address (page-aligned, user-space).
    pub vaddr: u64,
    /// Physical base address (page-aligned).
    /// For MAP_PHYS mappings, this is the caller-supplied physical address.
    pub phys: u64,
    /// Number of 4 KiB pages in this mapping.
    pub pages: u64,
    /// If true, the physical pages are owned by this process and will be
    /// freed to the buddy allocator on munmap/exit.  False for MAP_PHYS
    /// and FB_MAP where the physical memory is shared or device MMIO.
    pub owns_phys: bool,
}

impl Vma {
    pub const fn empty() -> Self {
        Self {
            vaddr: 0,
            phys: 0,
            pages: 0,
            owns_phys: false,
        }
    }

    /// A VMA slot is free if pages == 0.
    #[inline]
    pub const fn is_free(&self) -> bool {
        self.pages == 0
    }

    /// Total bytes covered by this mapping.
    #[inline]
    pub const fn size_bytes(&self) -> u64 {
        self.pages * 4096
    }

    /// Virtual address one past the end of this mapping.
    #[inline]
    pub const fn vaddr_end(&self) -> u64 {
        self.vaddr + self.pages * 4096
    }
}

/// Per-process VMA table.
///
/// Fixed-size, inline, zero-allocation.  Stored directly inside `Process`.
#[derive(Clone, Copy)]
pub struct VmaTable {
    entries: [Vma; MAX_VMAS],
}

impl VmaTable {
    pub const fn new() -> Self {
        Self {
            entries: [Vma::empty(); MAX_VMAS],
        }
    }

    /// Record a new mapping.  Returns `Ok(index)` or `Err` if the table is full.
    pub fn insert(&mut self, vaddr: u64, phys: u64, pages: u64, owns_phys: bool) -> Result<usize, ()> {
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.is_free() {
                *entry = Vma {
                    vaddr,
                    phys,
                    pages,
                    owns_phys,
                };
                return Ok(i);
            }
        }
        Err(())
    }

    /// Look up a VMA by its exact virtual base address.
    ///
    /// Returns `Some((index, &Vma))` if found.
    pub fn find_exact(&self, vaddr: u64) -> Option<(usize, &Vma)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_free() && entry.vaddr == vaddr {
                return Some((i, entry));
            }
        }
        None
    }

    /// Look up a VMA that contains the given virtual address.
    ///
    /// Returns `Some((index, &Vma))` if `vaddr` falls within any mapping.
    pub fn find_containing(&self, vaddr: u64) -> Option<(usize, &Vma)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_free() && vaddr >= entry.vaddr && vaddr < entry.vaddr_end() {
                return Some((i, entry));
            }
        }
        None
    }

    /// Remove and return the VMA at `index`.
    ///
    /// Returns the removed entry (for the caller to free physical pages).
    pub fn remove(&mut self, index: usize) -> Vma {
        let entry = self.entries[index];
        self.entries[index] = Vma::empty();
        entry
    }

    /// Iterate over all valid (non-free) VMAs.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Vma)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_free())
    }

    /// Number of active VMAs.
    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_free()).count()
    }
}
