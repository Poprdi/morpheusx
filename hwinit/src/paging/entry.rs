//! x86-64 Page Table Entry
//!
//! A page table entry is a 64-bit value that describes a single 4 KiB page
//! (or a 2 MiB / 1 GiB huge page when the PS bit is set).
//!
//! ```text
//! 63  52 51    12 11  9 8  7  6  5  4  3  2  1  0
//!  XD  AVL  ADDR  AVL  G  PS  D  A  CD WT  U  W  P
//! ```
//!
//! - **P**  — Present: entry is valid
//! - **W**  — Writable
//! - **U**  — User-mode accessible (CPL=3)
//! - **WT** — Write-through caching
//! - **CD** — Cache-disable
//! - **A**  — Accessed (set by CPU on read)
//! - **D**  — Dirty (set by CPU on write, page level only)
//! - **PS** — Page Size: 1 = 2 MiB (PD level) or 1 GiB (PDPT level)
//! - **G**  — Global: TLB entry survives CR3 reload (kernel pages)
//! - **XD** — Execute-Disable (NX bit, requires EFER.NXE=1)

/// x86-64 page table entry flags.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageFlags(pub u64);

impl PageFlags {
    // ── Core flags ───────────────────────────────────────────────────────
    pub const PRESENT:       Self = Self(1 << 0);
    pub const WRITABLE:      Self = Self(1 << 1);
    pub const USER:          Self = Self(1 << 2);
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    pub const CACHE_DISABLE: Self = Self(1 << 4);
    pub const ACCESSED:      Self = Self(1 << 5);
    pub const DIRTY:         Self = Self(1 << 6);
    pub const HUGE_PAGE:     Self = Self(1 << 7);   // PS bit
    pub const GLOBAL:        Self = Self(1 << 8);
    pub const NO_EXECUTE:    Self = Self(1 << 63);  // XD bit (EFER.NXE must be set)

    // ── Convenience presets ──────────────────────────────────────────────

    /// Kernel read-only page (present + global, NX).
    pub const KERNEL_RO: Self = Self(
        Self::PRESENT.0 | Self::GLOBAL.0 | Self::NO_EXECUTE.0
    );

    /// Kernel read-write page (present + writable + global, NX).
    pub const KERNEL_RW: Self = Self(
        Self::PRESENT.0 | Self::WRITABLE.0 | Self::GLOBAL.0 | Self::NO_EXECUTE.0
    );

    /// Kernel executable page (present + global, no NX).
    pub const KERNEL_CODE: Self = Self(
        Self::PRESENT.0 | Self::GLOBAL.0
    );

    /// User read-only page.
    pub const USER_RO: Self = Self(
        Self::PRESENT.0 | Self::USER.0 | Self::NO_EXECUTE.0
    );

    /// User read-write page.
    pub const USER_RW: Self = Self(
        Self::PRESENT.0 | Self::WRITABLE.0 | Self::USER.0 | Self::NO_EXECUTE.0
    );

    /// User executable page.
    pub const USER_CODE: Self = Self(
        Self::PRESENT.0 | Self::USER.0
    );

    /// Empty (not present).
    pub const EMPTY: Self = Self(0);

    // ── Combinators ──────────────────────────────────────────────────────

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[inline]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl core::fmt::Debug for PageFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut sep = false;
        let mut flag = |name: &'static str, bit: PageFlags| {
            if self.contains(bit) {
                if sep { let _ = f.write_str("|"); }
                let _ = f.write_str(name);
                sep = true;
            }
        };
        flag("P",  PageFlags::PRESENT);
        flag("W",  PageFlags::WRITABLE);
        flag("U",  PageFlags::USER);
        flag("WT", PageFlags::WRITE_THROUGH);
        flag("CD", PageFlags::CACHE_DISABLE);
        flag("A",  PageFlags::ACCESSED);
        flag("D",  PageFlags::DIRTY);
        flag("PS", PageFlags::HUGE_PAGE);
        flag("G",  PageFlags::GLOBAL);
        flag("XD", PageFlags::NO_EXECUTE);
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PAGE TABLE ENTRY
// ═══════════════════════════════════════════════════════════════════════════

/// A single 64-bit page table entry.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    /// Physical address mask (bits 12..51).
    const PHYS_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

    /// Create an empty (not-present) entry.
    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Create an entry pointing to `phys_frame` with the given flags.
    ///
    /// `phys_frame` must be 4 KiB-aligned; the lower 12 bits are masked off.
    #[inline]
    pub fn new(phys_frame: u64, flags: PageFlags) -> Self {
        Self((phys_frame & Self::PHYS_ADDR_MASK) | flags.0)
    }

    /// True if the Present bit is set.
    #[inline]
    pub fn is_present(self) -> bool {
        (self.0 & PageFlags::PRESENT.0) != 0
    }

    /// True if the Huge Page bit is set.
    #[inline]
    pub fn is_huge(self) -> bool {
        (self.0 & PageFlags::HUGE_PAGE.0) != 0
    }

    /// Physical frame address (bits 12..51 of the raw entry).
    #[inline]
    pub fn phys_addr(self) -> u64 {
        self.0 & Self::PHYS_ADDR_MASK
    }

    /// Raw 64-bit value.
    #[inline]
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Flags portion of the entry.
    #[inline]
    pub fn flags(self) -> PageFlags {
        PageFlags(self.0 & !Self::PHYS_ADDR_MASK)
    }

    /// Set Raw (for constructing intermediate table entries).
    #[inline]
    pub fn set_raw(&mut self, val: u64) {
        self.0 = val;
    }

    /// Clear the entry (make not present).
    #[inline]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl core::fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PTE(phys={:#x}, flags={:?})", self.phys_addr(), self.flags())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PAGE TABLE (one page = 512 entries)
// ═══════════════════════════════════════════════════════════════════════════

/// A single 4 KiB page table — 512 × 8-byte entries.
///
/// Used at every level: PML4, PDPT, PD, PT.
#[derive(Clone, Copy)]
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn empty() -> Self {
        Self {
            entries: [PageTableEntry::empty(); 512],
        }
    }

    /// Zero all entries.
    pub fn zero(&mut self) {
        for e in self.entries.iter_mut() {
            e.clear();
        }
    }

    /// Index operators
    #[inline]
    pub fn entry(&self, idx: usize) -> PageTableEntry {
        self.entries[idx & 511]
    }

    #[inline]
    pub fn entry_mut(&mut self, idx: usize) -> &mut PageTableEntry {
        &mut self.entries[idx & 511]
    }
}
