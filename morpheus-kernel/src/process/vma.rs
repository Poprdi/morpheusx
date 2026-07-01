//! Per-process VMA table for SYS_MMAP / SYS_MPROTECT / SYS_MAP_PHYS bookkeeping.
//! Fixed inline array, no heap. All addresses 4 KiB-aligned; entries do not
//! overlap; `owns_phys` decides whether MUNMAP frees the backing pages.

use morpheus_foundation::flags::PROT_WRITE;
use morpheus_foundation::PAGE_SIZE;

/// 128 (was 64): SYS_MPROTECT splits one VMA into up to three, so std setting a
/// guard page inside every thread-stack reservation triples the entry pressure.
pub const MAX_VMAS: usize = 128;

#[derive(Clone, Copy)]
pub struct Vma {
    pub vaddr: u64,
    /// For MAP_PHYS: the caller-supplied phys. For anonymous mmap: the contiguous
    /// backing block; a split sub-VMA points into it at `phys + (vaddr - base)`.
    pub phys: u64,
    pub pages: u64,
    /// False for MAP_PHYS / FB_MAP — only PTEs are unmapped.
    pub owns_phys: bool,
    /// Current protection (PROT_* bitmap) so a split can preserve the flanks.
    pub prot: u64,
    /// Task slot (tid) that created this mapping; `0` = leader/shared. Nonzero
    /// marks a thread-private map (stack/TLS) so `drain_owner` can reclaim it when
    /// that thread is reaped — otherwise a detached thread leaks it until process
    /// teardown. All threads' maps live in the leader's table (shared CR3).
    pub owner_tid: u32,
}

impl Vma {
    pub const fn empty() -> Self {
        Self {
            vaddr: 0,
            phys: 0,
            pages: 0,
            owns_phys: false,
            prot: 0,
            owner_tid: 0,
        }
    }

    #[inline]
    pub const fn is_free(&self) -> bool {
        self.pages == 0
    }

    #[inline]
    pub const fn size_bytes(&self) -> u64 {
        self.pages * PAGE_SIZE
    }

    #[inline]
    pub const fn vaddr_end(&self) -> u64 {
        self.vaddr + self.pages * PAGE_SIZE
    }

    /// Does this VMA overlap `[start, start + len_bytes)`?
    #[inline]
    pub fn overlaps(&self, start: u64, len_bytes: u64) -> bool {
        !self.is_free() && start < self.vaddr_end() && self.vaddr < start + len_bytes
    }
}

#[derive(Clone, Copy)]
pub struct VmaTable {
    entries: [Vma; MAX_VMAS],
}

impl Default for VmaTable {
    fn default() -> Self {
        Self::new()
    }
}

impl VmaTable {
    pub const fn new() -> Self {
        Self {
            entries: [Vma::empty(); MAX_VMAS],
        }
    }

    /// Insert defaulting `prot` to RW and `owner_tid` to 0 (leader/shared) — the
    /// contract non-mmap callers (MAP_PHYS, spawn image, fb) expose.
    pub fn insert(
        &mut self,
        vaddr: u64,
        phys: u64,
        pages: u64,
        owns_phys: bool,
    ) -> Result<usize, ()> {
        self.insert_full(vaddr, phys, pages, owns_phys, PROT_WRITE, 0)
    }

    /// `Err(())` if the table is full.
    pub fn insert_full(
        &mut self,
        vaddr: u64,
        phys: u64,
        pages: u64,
        owns_phys: bool,
        prot: u64,
        owner_tid: u32,
    ) -> Result<usize, ()> {
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.is_free() {
                *entry = Vma {
                    vaddr,
                    phys,
                    pages,
                    owns_phys,
                    prot,
                    owner_tid,
                };
                return Ok(i);
            }
        }
        Err(())
    }

    pub fn drain_owner(&mut self, owner_tid: u32, mut f: impl FnMut(&Vma)) {
        if owner_tid == 0 {
            return;
        }
        for entry in self.entries.iter_mut() {
            if !entry.is_free() && entry.owner_tid == owner_tid {
                f(entry);
                *entry = Vma::empty();
            }
        }
    }

    pub fn find_exact(&self, vaddr: u64) -> Option<(usize, &Vma)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_free() && entry.vaddr == vaddr {
                return Some((i, entry));
            }
        }
        None
    }

    pub fn find_containing(&self, vaddr: u64) -> Option<(usize, &Vma)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_free() && vaddr >= entry.vaddr && vaddr < entry.vaddr_end() {
                return Some((i, entry));
            }
        }
        None
    }

    /// Index of the single VMA that fully contains `[vaddr, vaddr + pages)`.
    /// `None` if the range straddles a hole or VMA boundary (POSIX mprotect on a
    /// partially-unmapped range is an error).
    pub fn find_containing_range(&self, vaddr: u64, pages: u64) -> Option<usize> {
        let end = vaddr + pages * PAGE_SIZE;
        for (i, entry) in self.entries.iter().enumerate() {
            if !entry.is_free() && vaddr >= entry.vaddr && end <= entry.vaddr_end() {
                return Some(i);
            }
        }
        None
    }

    /// Any VMA overlapping `[start, start + pages)`?
    pub fn overlaps_any(&self, start: u64, pages: u64) -> bool {
        let len = pages * PAGE_SIZE;
        self.entries.iter().any(|e| e.overlaps(start, len))
    }

    /// First-fit free VA hole of `pages` in `[base, limit)`. Walks occupied
    /// regions, jumping the candidate past each overlap — so freed VMAs leave
    /// reusable gaps (no monotonic-bump VA leak). Bounded by MAX_VMAS+1 probes.
    pub fn find_free_va(&self, base: u64, limit: u64, pages: u64) -> Option<u64> {
        let len = pages * PAGE_SIZE;
        let mut candidate = base;
        for _ in 0..=MAX_VMAS {
            if candidate.checked_add(len)? > limit {
                return None;
            }
            match self
                .entries
                .iter()
                .filter(|e| e.overlaps(candidate, len))
                .map(|e| e.vaddr_end())
                .max()
            {
                Some(next) => candidate = next,
                None => return Some(candidate),
            }
        }
        None
    }

    pub fn get(&self, index: usize) -> Vma {
        self.entries[index]
    }

    pub fn set_at(&mut self, index: usize, vma: Vma) {
        self.entries[index] = vma;
    }

    /// Free slots currently available.
    pub fn free_slots(&self) -> usize {
        self.entries.iter().filter(|e| e.is_free()).count()
    }

    /// Caller is responsible for freeing physical pages.
    pub fn remove(&mut self, index: usize) -> Vma {
        let entry = self.entries[index];
        self.entries[index] = Vma::empty();
        entry
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Vma)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_free())
    }

    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_free()).count()
    }
}
