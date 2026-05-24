//! Per-process VMA table for SYS_MMAP / SYS_MAP_PHYS bookkeeping.
//! Fixed inline array, no heap. All addresses 4 KiB-aligned; entries do not
//! overlap; `owns_phys` decides whether MUNMAP frees the backing pages.

pub const MAX_VMAS: usize = 64;

#[derive(Clone, Copy)]
pub struct Vma {
    pub vaddr: u64,
    /// For MAP_PHYS: the caller-supplied phys.
    pub phys: u64,
    pub pages: u64,
    /// False for MAP_PHYS / FB_MAP — only PTEs are unmapped.
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

    #[inline]
    pub const fn is_free(&self) -> bool {
        self.pages == 0
    }

    #[inline]
    pub const fn size_bytes(&self) -> u64 {
        self.pages * crate::memory::PAGE_SIZE
    }

    #[inline]
    pub const fn vaddr_end(&self) -> u64 {
        self.vaddr + self.pages * crate::memory::PAGE_SIZE
    }
}

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

    /// `Err(())` if the table is full.
    pub fn insert(
        &mut self,
        vaddr: u64,
        phys: u64,
        pages: u64,
        owns_phys: bool,
    ) -> Result<usize, ()> {
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
