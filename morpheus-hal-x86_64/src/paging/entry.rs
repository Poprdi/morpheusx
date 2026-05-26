//! AMD64 Vol 2 §5.4 page-table entry bits.
//! Layout: `[63 XD][62..52 AVL][51..12 ADDR][11..9 AVL][8 G][7 PS][6 D][5 A][4 CD][3 WT][2 U][1 W][0 P]`.

/// x86-64 PTE flags.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageFlags(pub u64);

impl PageFlags {
    pub const PRESENT: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const USER: Self = Self(1 << 2);
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    pub const CACHE_DISABLE: Self = Self(1 << 4);
    pub const ACCESSED: Self = Self(1 << 5);
    pub const DIRTY: Self = Self(1 << 6);
    /// PS bit: 2 MiB (PD) or 1 GiB (PDPT).
    pub const HUGE_PAGE: Self = Self(1 << 7);
    pub const GLOBAL: Self = Self(1 << 8);
    /// XD; requires EFER.NXE=1.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    pub const KERNEL_RO: Self = Self(Self::PRESENT.0 | Self::GLOBAL.0 | Self::NO_EXECUTE.0);
    pub const KERNEL_RW: Self =
        Self(Self::PRESENT.0 | Self::WRITABLE.0 | Self::GLOBAL.0 | Self::NO_EXECUTE.0);
    pub const KERNEL_CODE: Self = Self(Self::PRESENT.0 | Self::GLOBAL.0);
    pub const USER_RO: Self = Self(Self::PRESENT.0 | Self::USER.0 | Self::NO_EXECUTE.0);
    pub const USER_RW: Self =
        Self(Self::PRESENT.0 | Self::WRITABLE.0 | Self::USER.0 | Self::NO_EXECUTE.0);
    pub const USER_CODE: Self = Self(Self::PRESENT.0 | Self::USER.0);
    pub const EMPTY: Self = Self(0);

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
                if sep {
                    let _ = f.write_str("|");
                }
                let _ = f.write_str(name);
                sep = true;
            }
        };
        flag("P", PageFlags::PRESENT);
        flag("W", PageFlags::WRITABLE);
        flag("U", PageFlags::USER);
        flag("WT", PageFlags::WRITE_THROUGH);
        flag("CD", PageFlags::CACHE_DISABLE);
        flag("A", PageFlags::ACCESSED);
        flag("D", PageFlags::DIRTY);
        flag("PS", PageFlags::HUGE_PAGE);
        flag("G", PageFlags::GLOBAL);
        flag("XD", PageFlags::NO_EXECUTE);
        Ok(())
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    /// Bits 12..51.
    const PHYS_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Low 12 bits of `phys_frame` are masked off.
    #[inline]
    pub fn new(phys_frame: u64, flags: PageFlags) -> Self {
        Self((phys_frame & Self::PHYS_ADDR_MASK) | flags.0)
    }

    #[inline]
    pub fn is_present(self) -> bool {
        (self.0 & PageFlags::PRESENT.0) != 0
    }

    #[inline]
    pub fn is_huge(self) -> bool {
        (self.0 & PageFlags::HUGE_PAGE.0) != 0
    }

    #[inline]
    pub fn phys_addr(self) -> u64 {
        self.0 & Self::PHYS_ADDR_MASK
    }

    #[inline]
    pub fn raw(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn flags(self) -> PageFlags {
        PageFlags(self.0 & !Self::PHYS_ADDR_MASK)
    }

    /// For building intermediate table entries.
    #[inline]
    pub fn set_raw(&mut self, val: u64) {
        self.0 = val;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

impl core::fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "PTE(phys={:#x}, flags={:?})",
            self.phys_addr(),
            self.flags()
        )
    }
}

/// 512 × 8-byte entries; used at PML4/PDPT/PD/PT.
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

    pub fn zero(&mut self) {
        for e in self.entries.iter_mut() {
            e.clear();
        }
    }

    #[inline]
    pub fn entry(&self, idx: usize) -> PageTableEntry {
        self.entries[idx & 511]
    }

    #[inline]
    pub fn entry_mut(&mut self, idx: usize) -> &mut PageTableEntry {
        &mut self.entries[idx & 511]
    }
}
