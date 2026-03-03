//! Physical memory buddy allocator.
//! Intrusive free-lists, XOR buddy math, O(log N) alloc/free.
//! MAX_ORDER = 26 → 256 GiB max block. change one constant for more.
//! No bitmap, no per-page array, ~13 KiB BSS. the dream.

use crate::serial::{put_hex32, put_hex64, puts};

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;

/// one constant to rule them all. 26 → 256 GiB, 28 → 1 TiB, 30 → 4 TiB.
const MAX_ORDER: usize = 26;

const MAX_MAP: usize = 384; // UEFI snapshot slots

// memory types (UEFI + our own tags)

/// UEFI EFI_MEMORY_TYPE + custom allocator tags in the 0x8000_xxxx range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryType {
    Reserved = 0,
    LoaderCode = 1,
    LoaderData = 2,
    BootServicesCode = 3,
    BootServicesData = 4,
    RuntimeServicesCode = 5,
    RuntimeServicesData = 6,
    Conventional = 7,
    Unusable = 8,
    AcpiReclaim = 9,
    AcpiNvs = 10,
    Mmio = 11,
    MmioPortSpace = 12,
    PalCode = 13,
    Persistent = 14,

    // custom tags (0x8000_xxxx range, safe from UEFI collisions)
    AllocatedDma = 0x8000_0001,
    AllocatedStack = 0x8000_0002,
    AllocatedPageTable = 0x8000_0003,
    AllocatedHeap = 0x8000_0004,
    Allocated = 0x8000_0000,
}

impl MemoryType {
    pub fn from_uefi_raw(value: u32) -> Self {
        match value {
            0 => Self::Reserved,
            1 => Self::LoaderCode,
            2 => Self::LoaderData,
            3 => Self::BootServicesCode,
            4 => Self::BootServicesData,
            5 => Self::RuntimeServicesCode,
            6 => Self::RuntimeServicesData,
            7 => Self::Conventional,
            8 => Self::Unusable,
            9 => Self::AcpiReclaim,
            10 => Self::AcpiNvs,
            11 => Self::Mmio,
            12 => Self::MmioPortSpace,
            13 => Self::PalCode,
            14 => Self::Persistent,
            _ => Self::Reserved,
        }
    }

    /// Free to use right now. we intentionally exclude BootServices{Code,Data}
    /// because EDK2 still has WP bits set and writing FreeNode into them = #PF.
    pub fn is_free(&self) -> bool {
        matches!(
            self,
            Self::Conventional | Self::LoaderCode | Self::LoaderData
        )
    }

    /// same as is_free(). exists so call sites read better.
    pub fn is_immediately_writable(&self) -> bool {
        self.is_free()
    }

    pub fn is_reclaimable(&self) -> bool {
        matches!(self, Self::AcpiReclaim)
    }

    pub fn must_preserve(&self) -> bool {
        matches!(
            self,
            Self::Reserved
                | Self::RuntimeServicesCode
                | Self::RuntimeServicesData
                | Self::AcpiNvs
                | Self::Mmio
                | Self::MmioPortSpace
                | Self::PalCode
                | Self::Unusable
        )
    }

    pub fn to_e820(&self) -> E820Type {
        match self {
            Self::Conventional
            | Self::LoaderCode
            | Self::LoaderData
            | Self::BootServicesCode   // free after EBS — usable RAM in E820
            | Self::BootServicesData   // free after EBS — usable RAM in E820
            | Self::Allocated
            | Self::AllocatedDma
            | Self::AllocatedStack
            | Self::AllocatedPageTable
            | Self::AllocatedHeap => E820Type::Ram,

            Self::AcpiReclaim => E820Type::Acpi,
            Self::AcpiNvs     => E820Type::Nvs,
            Self::Persistent  => E820Type::Pmem,
            Self::Unusable    => E820Type::Unusable,
            _                 => E820Type::Reserved,
        }
    }
}

// memory attributes (UEFI verbatim)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAttribute(pub u64);

impl MemoryAttribute {
    pub const UC: Self = Self(0x0000_0000_0000_0001);
    pub const WC: Self = Self(0x0000_0000_0000_0002);
    pub const WT: Self = Self(0x0000_0000_0000_0004);
    pub const WB: Self = Self(0x0000_0000_0000_0008);
    pub const UCE: Self = Self(0x0000_0000_0000_0010);
    pub const WP: Self = Self(0x0000_0000_0000_1000);
    pub const RP: Self = Self(0x0000_0000_0000_2000);
    pub const XP: Self = Self(0x0000_0000_0000_4000);
    pub const NV: Self = Self(0x0000_0000_0000_8000);
    pub const MORE_RELIABLE: Self = Self(0x0000_0000_0001_0000);
    pub const RO: Self = Self(0x0000_0000_0002_0000);
    pub const SP: Self = Self(0x0000_0000_0004_0000);
    pub const RUNTIME: Self = Self(0x8000_0000_0000_0000);

    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn contains(self, o: Self) -> bool {
        (self.0 & o.0) == o.0
    }
    pub const fn union(self, o: Self) -> Self {
        Self(self.0 | o.0)
    }
}

// e820 (linux handoff)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum E820Type {
    Ram = 1,
    Reserved = 2,
    Acpi = 3,
    Nvs = 4,
    Unusable = 5,
    Disabled = 6,
    Pmem = 7,
    Undefined = 8,
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub entry_type: u32,
}

// UEFI EFI_MEMORY_DESCRIPTOR verbatim

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub mem_type: MemoryType,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: MemoryAttribute,
}

impl MemoryDescriptor {
    pub const fn empty() -> Self {
        Self {
            mem_type: MemoryType::Reserved,
            physical_start: 0,
            virtual_start: 0,
            number_of_pages: 0,
            attribute: MemoryAttribute::empty(),
        }
    }
    pub const fn physical_end(&self) -> u64 {
        self.physical_start + self.number_of_pages * PAGE_SIZE
    }
    pub const fn size(&self) -> u64 {
        self.number_of_pages * PAGE_SIZE
    }
    pub const fn contains(&self, addr: u64) -> bool {
        addr >= self.physical_start && addr < self.physical_end()
    }
}

// allocation types (mirrors UEFI)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateType {
    /// Any free pages.
    AnyPages,
    /// Highest address whose end ≤ specified limit (needed for DMA < 4 GB).
    MaxAddress(u64),
    /// Exactly this physical address (page-aligned).
    Address(u64),
}

// errors

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    OutOfResources,
    InvalidParameter,
    NotFound,
    BufferTooSmall,
    AlreadyAllocated,
}

// intrusive free-list node — 16 bytes crammed into each free page

#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

/// the buddy allocator. public name is MemoryRegistry, don't rename it.
pub struct MemoryRegistry {
    free_lists: [*mut FreeNode; MAX_ORDER + 1],
    free_at_order: [u64; MAX_ORDER + 1],
    total_pages: u64,
    free_pages: u64,
    allocated_pages: u64,
    map_key: u64, // monotonic, callers detect changes

    /// UEFI snapshot. only for type queries and E820 export, not allocation.
    map: [MemoryDescriptor; MAX_MAP],
    map_count: usize,

    /// PE-image exclusion zone saved from initial import — used by reclaim pass.
    excl_start: u64,
    excl_end: u64,
}

// no SMP, bare-metal, identity-mapped. these are fine.
unsafe impl Send for MemoryRegistry {}
unsafe impl Sync for MemoryRegistry {}

impl MemoryRegistry {
    /// Create a zeroed, empty registry.  `const` so it fits in BSS.
    pub const fn new() -> Self {
        Self {
            free_lists: [core::ptr::null_mut(); MAX_ORDER + 1],
            free_at_order: [0; MAX_ORDER + 1],
            total_pages: 0,
            free_pages: 0,
            allocated_pages: 0,
            map_key: 0,
            map: [MemoryDescriptor::empty(); MAX_MAP],
            map_count: 0,
            excl_start: 0,
            excl_end: 0,
        }
    }

    // init (once, after ExitBootServices, don't call again or you'll hurt)

    /// Parse UEFI memory map into buddy free lists.
    /// exclude_base/exclude_pages: PE image range that must NOT become free
    /// (the buddy would scribble FreeNode into our own .bss — ask me how I know).
    /// Pass (0,0) for no exclusion.
    ///
    /// # Safety
    /// `map_ptr` must point to a valid UEFI memory map of `map_size` bytes
    /// with entries spaced `descriptor_size` bytes apart.
    /// Must be called exactly once, single-threaded, before any allocation.
    /// `hw_holes` is a **sorted** slice of page-aligned physical addresses
    /// that must never be added to the buddy.  Typically: live page-table
    /// pages, GDT page, IDT pages.  Writing FreeNode into any of these
    /// corrupts the corresponding CPU structure — instant #GP or #PF.
    pub unsafe fn import_uefi_map(
        &mut self,
        map_ptr: *const u8,
        map_size: usize,
        descriptor_size: usize,
        _descriptor_version: u32,
        exclude_base: u64,
        exclude_pages: u64,
        hw_holes: &[u64],
    ) {
        let excl_start = exclude_base;
        let excl_end = exclude_base + exclude_pages * PAGE_SIZE;
        self.excl_start = excl_start;
        self.excl_end = excl_end;
        let entry_count = map_size / descriptor_size;

        for i in 0..entry_count {
            let ptr = map_ptr.add(i * descriptor_size);

            // UEFI EFI_MEMORY_DESCRIPTOR (v1) offsets:
            //   0  : u32 Type
            //   4  : u32 pad
            //   8  : u64 PhysicalStart
            //  16  : u64 VirtualStart
            //  24  : u64 NumberOfPages
            //  32  : u64 Attribute
            let raw_type = *(ptr as *const u32);
            let phys = *(ptr.add(8) as *const u64);
            let virt = *(ptr.add(16) as *const u64);
            let pages = *(ptr.add(24) as *const u64);
            let attr = *(ptr.add(32) as *const u64);

            if pages == 0 {
                continue;
            }

            let mem_type = MemoryType::from_uefi_raw(raw_type);

            // Snapshot into map for type queries / E820.
            if self.map_count < MAX_MAP {
                self.map[self.map_count] = MemoryDescriptor {
                    mem_type,
                    physical_start: phys,
                    virtual_start: virt,
                    number_of_pages: pages,
                    attribute: MemoryAttribute(attr),
                };
                self.map_count += 1;
            }

            self.total_pages += pages;

            if mem_type.is_immediately_writable() {
                // only Conventional/LoaderCode/LoaderData are safe to touch NOW.
                // BootServices pages still have WP bits = instant #PF.
                // also skip first 1 MiB (BIOS land, here be dragons).
                let region_end = phys + pages * PAGE_SIZE;
                let start = if phys < 0x10_0000 { 0x10_0000 } else { phys };
                if start < region_end {
                    // First clip around PE image, then hole-punch hw_holes
                    // (page-table pages, GDT, IDT) so the buddy never
                    // writes FreeNode into live CPU structures.
                    if excl_end > excl_start && start < excl_end && region_end > excl_start {
                        if start < excl_start {
                            self.add_range_punching_holes(start, excl_start, hw_holes);
                        }
                        if region_end > excl_end {
                            self.add_range_punching_holes(excl_end, region_end, hw_holes);
                        }
                    } else {
                        self.add_range_punching_holes(start, region_end, hw_holes);
                    }
                }
            }
        }

        self.map_key = 1;
        self.print_summary();
    }

    /// punch a hole in the buddy free lists. protect page tables, stacks, etc.
    pub unsafe fn reserve_range(&mut self, base: u64, pages: u64) {
        let mut addr = base & !(PAGE_SIZE - 1);
        let end = addr + pages * PAGE_SIZE;
        while addr < end {
            self.buddy_remove_page(addr);
            addr += PAGE_SIZE;
        }
    }

    // allocation API

    /// allocate_pages. the one function that matters.
    pub fn allocate_pages(
        &mut self,
        alloc_type: AllocateType,
        mem_type: MemoryType,
        pages: u64,
    ) -> Result<u64, MemoryError> {
        if pages == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        let order = pages_to_order(pages);

        // Buddy free-list nodes live at identity-mapped physical addresses.
        // Under a user CR3 those VAs may be remapped — switch to kernel CR3.
        let addr = {
            let _guard = unsafe { KernelCr3Guard::enter() };
            match alloc_type {
                AllocateType::AnyPages => self.buddy_alloc(order)?,
                AllocateType::MaxAddress(limit) => self.buddy_alloc_below(order, limit)?,
                AllocateType::Address(want) => self.buddy_alloc_at(want, pages)?,
            }
        };

        self.map_snapshot_add(addr, pages, mem_type);
        self.map_key += 1;
        Ok(addr)
    }

    /// free_pages. decomposes into aligned buddy blocks, coalesces automatically.
    pub fn free_pages(&mut self, addr: u64, pages: u64) -> Result<(), MemoryError> {
        // Note: free_pages counter is now tracked entirely by list_push/remove/pop.
        if addr & (PAGE_SIZE - 1) != 0 || pages == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        self.map_snapshot_remove(addr, pages);

        // Switch to kernel CR3 for buddy free-list manipulation.
        {
            let _guard = unsafe { KernelCr3Guard::enter() };
            let mut base = addr;
            let end = addr + pages * PAGE_SIZE;
            while base < end {
                let remaining_pages = (end - base) / PAGE_SIZE;
                let align_order =
                    (base.trailing_zeros() as usize).saturating_sub(PAGE_SHIFT as usize);
                let size_order =
                    usize::BITS as usize - 1 - remaining_pages.leading_zeros() as usize;
                let order = align_order.min(size_order).min(MAX_ORDER);
                unsafe {
                    self.buddy_free(base, order);
                }
                base += (1u64 << order) * PAGE_SIZE;
            }
        }

        self.map_key += 1;
        Ok(())
    }

    /// byte-granular alloc. rounds up to pages because that's all we have.
    pub fn allocate_pool(&mut self, size: usize) -> Result<u64, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidParameter);
        }
        let pages = (size as u64).div_ceil(PAGE_SIZE);
        self.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, pages)
    }

    // query api

    pub fn get_memory_map(&self) -> (u64, usize) {
        (self.map_key, self.map_count)
    }
    pub fn get_map_key(&self) -> u64 {
        self.map_key
    }

    pub fn get_descriptor(&self, index: usize) -> Option<&MemoryDescriptor> {
        if index < self.map_count {
            Some(&self.map[index])
        } else {
            None
        }
    }

    pub fn total_memory(&self) -> u64 {
        self.total_pages * PAGE_SIZE
    }
    pub fn free_memory(&self) -> u64 {
        self.free_pages.saturating_mul(PAGE_SIZE)
    }

    /// Reclaim UEFI BootServices memory into the buddy allocator.
    ///
    /// Call once, well after ExitBootServices — after GDT/IDT/PIC/heap/TSC/
    /// paging/scheduler init so that UEFI boot-time services are truly gone.
    /// Immediately call `reserve_page_table_pages()` after this to re-lock
    /// any page-table pages that were in BootServices address space.
    ///
    /// Applies the same first-1MiB skip and PE-image exclusion as the initial
    /// import, so the bootloader image is never corrupted.
    ///
    /// `cpu_excl` is a **sorted** array of page-aligned physical addresses
    /// that must NOT be added to the buddy — live page-table pages, GDT,
    /// IDT, etc.  Writing FreeNode into these would corrupt the active
    /// PML4 / PDPT / PD / PT / GDT / IDT — instant #GP or #PF.
    ///
    /// # Safety
    /// - `import_uefi_map` must have been called first.
    /// - No UEFI boot services may be invoked after this returns.
    /// - Single-threaded context only.
    pub unsafe fn reclaim_boot_services(&mut self, cpu_excl: &[u64]) {
        let excl_start = self.excl_start;
        let excl_end = self.excl_end;
        let mut reclaimed_pages: u64 = 0;

        for i in 0..self.map_count {
            let d = &self.map[i];
            if !matches!(
                d.mem_type,
                MemoryType::BootServicesCode | MemoryType::BootServicesData
            ) {
                continue;
            }

            let phys = d.physical_start;
            let pages = d.number_of_pages;
            if pages == 0 {
                continue;
            }

            let region_end = phys + pages * PAGE_SIZE;

            // Skip first 1 MiB (BIOS land, VGA, ROM shadows).
            let start = if phys < 0x10_0000 { 0x10_0000 } else { phys };
            if start >= region_end {
                continue;
            }

            // Apply PE-image exclusion, then cpu_excl hole-punch.
            // Build a list of safe sub-ranges after PE exclusion,
            // then hole-punch each one for cpu_excl pages.
            let mut subs: [(u64, u64); 2] = [(0, 0); 2];
            let mut nsub = 0usize;

            if excl_end > excl_start && start < excl_end && region_end > excl_start {
                if start < excl_start {
                    subs[nsub] = (start, excl_start);
                    nsub += 1;
                }
                if region_end > excl_end {
                    subs[nsub] = (excl_end, region_end);
                    nsub += 1;
                }
            } else {
                subs[0] = (start, region_end);
                nsub = 1;
            }

            for s in 0..nsub {
                let (s_start, s_end) = subs[s];
                reclaimed_pages += self.add_range_punching_holes(s_start, s_end, cpu_excl);
            }
        }

        let reclaimed_mb = (reclaimed_pages * PAGE_SIZE) >> 20;
        puts("[MEM] reclaim_boot_services: freed ");
        put_hex32(reclaimed_mb as u32);
        puts(" MB  free_total=");
        put_hex32(((self.free_pages * PAGE_SIZE) >> 20) as u32);
        puts(" MB\n");
    }

    /// Add range [start, end) to the buddy, skipping any pages in `holes`.
    /// `holes` must be sorted ascending.  Returns number of pages added.
    unsafe fn add_range_punching_holes(&mut self, start: u64, end: u64, holes: &[u64]) -> u64 {
        let mut added = 0u64;
        let mut cur = start;

        for &hole in holes {
            if hole >= end {
                break;
            }
            if hole < cur {
                continue;
            }
            // Add [cur, hole)
            if cur < hole {
                let pages = (hole - cur) / PAGE_SIZE;
                self.buddy_add_range(cur, hole);
                added += pages;
            }
            // Skip the hole page
            cur = hole + PAGE_SIZE;
        }

        // Remainder after all holes
        if cur < end {
            let pages = (end - cur) / PAGE_SIZE;
            self.buddy_add_range(cur, end);
            added += pages;
        }

        added
    }

    pub fn allocated_memory(&self) -> u64 {
        self.allocated_pages.saturating_mul(PAGE_SIZE)
    }

    /// bump space remaining. always 0 because bump is dead. long live buddy.
    pub fn bump_remaining(&self) -> u64 {
        0
    }

    /// Find the largest free region entirely below 4 GiB.
    pub fn find_largest_free_below_4gb(&self) -> Option<(u64, u64)> {
        for order in (0..=MAX_ORDER).rev() {
            let block_bytes = (1u64 << order) * PAGE_SIZE;
            let mut cur = self.free_lists[order];
            while !cur.is_null() {
                let base = cur as u64;
                if base.saturating_add(block_bytes) <= 0x1_0000_0000 {
                    return Some((base, block_bytes));
                }
                // SAFETY: node was written by list_push; identity-mapped.
                unsafe {
                    cur = (*cur).next;
                }
            }
        }
        None
    }

    /// Memory type at a given physical address (snapshot only).
    pub fn memory_type_at(&self, addr: u64) -> MemoryType {
        for i in 0..self.map_count {
            if self.map[i].contains(addr) {
                return self.map[i].mem_type;
            }
        }
        MemoryType::Reserved
    }

    /// Write E820 entries into `buffer`, return count written.
    pub fn export_e820(&self, buffer: &mut [E820Entry]) -> usize {
        let n = self.map_count.min(buffer.len());
        for (i, slot) in buffer.iter_mut().enumerate().take(n) {
            let d = &self.map[i];
            *slot = E820Entry {
                addr: d.physical_start,
                size: d.size(),
                entry_type: d.mem_type.to_e820() as u32,
            };
        }
        n
    }

    pub fn e820_count(&self) -> usize {
        self.map_count
    }

    // convenience allocators

    /// DMA-safe pages (physical address + size ≤ 4 GiB).
    pub fn alloc_dma_pages(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(
            AllocateType::MaxAddress(0xFFFF_FFFF),
            MemoryType::AllocatedDma,
            pages,
        )
    }

    /// DMA-safe bytes (page-rounded, below 4 GiB).
    pub fn alloc_dma_bytes(&mut self, size: usize) -> Result<u64, MemoryError> {
        let pages = (size as u64).div_ceil(PAGE_SIZE);
        self.alloc_dma_pages(pages)
    }

    /// Kernel stack pages (any address).
    pub fn alloc_stack(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedStack, pages)
    }

    // buddy internals

    /// Check if a pointer is a canonical x86-64 address.
    /// Non-canonical addresses (bits 48–63 not sign-extended from bit 47)
    /// trigger #GP on dereference. Detects OVMF 0xAFAFAFAF scrub poison.
    #[inline]
    fn is_canonical(ptr: *mut FreeNode) -> bool {
        if ptr.is_null() {
            return true; // null is fine, callers check it separately
        }
        let addr = ptr as u64;
        let top17 = addr >> 47;
        top17 == 0 || top17 == 0x1FFFF
    }

    /// Push a block at `addr` onto free_lists[order].
    ///
    /// # Safety: addr must be 4-KiB aligned and physically mapped.
    unsafe fn list_push(&mut self, addr: u64, order: usize) {
        let node = addr as *mut FreeNode;
        let old_head = self.free_lists[order];
        if !Self::is_canonical(old_head) {
            puts("[MEM] CORRUPT list_push: head=");
            put_hex64(old_head as u64);
            puts(" order=");
            put_hex32(order as u32);
            puts(" — starting fresh chain\n");
            self.free_lists[order] = node;
            (*node).next = core::ptr::null_mut();
            (*node).prev = core::ptr::null_mut();
            self.free_at_order[order] = 1;
            self.free_pages += 1u64 << order;
            return;
        }
        (*node).next = old_head;
        (*node).prev = core::ptr::null_mut();
        if !old_head.is_null() {
            (*old_head).prev = node;
        }
        self.free_lists[order] = node;
        self.free_at_order[order] += 1;
        self.free_pages += 1u64 << order;
    }

    /// Remove the specific block at `addr` from free_lists[order].
    /// Returns true if found.
    unsafe fn list_remove(&mut self, addr: u64, order: usize) -> bool {
        let target = addr as *mut FreeNode;
        let mut cur = self.free_lists[order];
        while !cur.is_null() {
            if cur == target {
                let prev = (*cur).prev;
                let next = (*cur).next;
                // Validate pointers before unlinking.
                if !Self::is_canonical(prev) || !Self::is_canonical(next) {
                    puts("[MEM] CORRUPT list_remove unlink: node=");
                    put_hex64(cur as u64);
                    puts(" prev=");
                    put_hex64(prev as u64);
                    puts(" next=");
                    put_hex64(next as u64);
                    puts(" order=");
                    put_hex32(order as u32);
                    puts("\n");
                    // Sever the chain at this node.
                    self.free_lists[order] = core::ptr::null_mut();
                    self.free_at_order[order] = 0;
                    // Don't adjust free_pages here — chain state is unknown.
                    return true;
                }
                if !prev.is_null() {
                    (*prev).next = next;
                } else {
                    self.free_lists[order] = next;
                }
                if !next.is_null() {
                    (*next).prev = prev;
                }
                self.free_at_order[order] -= 1;
                self.free_pages = self.free_pages.saturating_sub(1u64 << order);
                return true;
            }
            let next = (*cur).next;
            if !Self::is_canonical(next) {
                puts("[MEM] CORRUPT list_remove walk: node=");
                put_hex64(cur as u64);
                puts(" next=");
                put_hex64(next as u64);
                puts(" order=");
                put_hex32(order as u32);
                puts(" target=");
                put_hex64(addr);
                puts("\n");
                // Terminate the corrupted chain here.
                (*cur).next = core::ptr::null_mut();
                break;
            }
            cur = next;
        }
        false
    }

    /// Pop the head block from free_lists[order].
    unsafe fn list_pop(&mut self, order: usize) -> Option<u64> {
        let head = self.free_lists[order];
        if head.is_null() {
            return None;
        }
        if !Self::is_canonical(head) {
            puts("[MEM] CORRUPT list_pop: head=");
            put_hex64(head as u64);
            puts(" order=");
            put_hex32(order as u32);
            puts("\n");
            self.free_lists[order] = core::ptr::null_mut();
            self.free_at_order[order] = 0;
            // Don't adjust free_pages — chain state is unknown.
            return None;
        }
        let next = (*head).next;
        if !Self::is_canonical(next) {
            puts("[MEM] CORRUPT list_pop next: head=");
            put_hex64(head as u64);
            puts(" next=");
            put_hex64(next as u64);
            puts(" order=");
            put_hex32(order as u32);
            puts("\n");
            self.free_lists[order] = core::ptr::null_mut();
            self.free_at_order[order] = 0;
            // Deduct just this one block since we're returning it.
            self.free_pages = self.free_pages.saturating_sub(1u64 << order);
            return Some(head as u64);
        }
        self.free_lists[order] = next;
        if !next.is_null() {
            (*next).prev = core::ptr::null_mut();
        }
        self.free_at_order[order] -= 1;
        self.free_pages = self.free_pages.saturating_sub(1u64 << order);
        Some(head as u64)
    }

    /// Buddy address of `addr` at `order`.
    #[inline(always)]
    fn buddy_of(addr: u64, order: usize) -> u64 {
        addr ^ ((1u64 << order) * PAGE_SIZE)
    }

    /// Allocate a 2^order-page block from any address.
    fn buddy_alloc(&mut self, order: usize) -> Result<u64, MemoryError> {
        // Find smallest k ≥ order with a free block.
        let top = (order..=MAX_ORDER)
            .find(|&k| !self.free_lists[k].is_null())
            .ok_or(MemoryError::OutOfResources)?;

        let addr = unsafe { self.list_pop(top).unwrap() };

        // Split from `top` down to `order`, pushing spare halves back.
        // list_pop already decremented free_pages by 1<<top;
        // list_push will increment it for each spare → net -1<<order. No manual adj.
        let cur = addr;
        let mut cur_k = top;
        while cur_k > order {
            cur_k -= 1;
            let spare = cur + (1u64 << cur_k) * PAGE_SIZE;
            unsafe {
                self.list_push(spare, cur_k);
            }
        }

        self.allocated_pages += 1u64 << order;
        Ok(cur)
    }

    /// Allocate a 2^order-page block whose range is entirely ≤ `limit`.
    fn buddy_alloc_below(&mut self, order: usize, limit: u64) -> Result<u64, MemoryError> {
        let block_bytes = (1u64 << order) * PAGE_SIZE;

        // Scan from highest order down — prefer fewer splits.
        for k in (order..=MAX_ORDER).rev() {
            let k_bytes = (1u64 << k) * PAGE_SIZE;
            let mut cur = self.free_lists[k];
            while !cur.is_null() {
                let base = cur as u64;
                let base_end = base.saturating_add(k_bytes);
                if base.saturating_add(block_bytes) <= limit.saturating_add(1)
                    && base_end <= limit.saturating_add(1)
                {
                    // Carve `order` pages from this block.
                    // list_remove decrements free_pages by 1<<k;
                    // list_push increments it for each spare → net -1<<order.
                    unsafe {
                        self.list_remove(base, k);
                    }

                    let current = base;
                    let mut current_k = k;
                    while current_k > order {
                        current_k -= 1;
                        let spare = current + (1u64 << current_k) * PAGE_SIZE;
                        unsafe {
                            self.list_push(spare, current_k);
                        }
                    }

                    self.allocated_pages += 1u64 << order;
                    return Ok(current);
                }
                unsafe {
                    cur = (*cur).next;
                }
            }
        }
        Err(MemoryError::OutOfResources)
    }

    /// Allocate exactly `pages` pages at a fixed physical address.
    fn buddy_alloc_at(&mut self, addr: u64, pages: u64) -> Result<u64, MemoryError> {
        if addr & (PAGE_SIZE - 1) != 0 {
            return Err(MemoryError::InvalidParameter);
        }
        let end = addr + pages * PAGE_SIZE;
        let mut base = addr;
        while base < end {
            let remaining = (end - base) / PAGE_SIZE;
            let align_order = (base.trailing_zeros() as usize).saturating_sub(PAGE_SHIFT as usize);
            let size_order = usize::BITS as usize - 1 - remaining.leading_zeros() as usize;
            let order = align_order.min(size_order).min(MAX_ORDER);
            // SAFETY: identity-mapped physical range.
            unsafe {
                self.carve_block(base, order);
            }
            self.allocated_pages += 1u64 << order;
            // carve_block handles free_pages via list_remove/push.
            base += (1u64 << order) * PAGE_SIZE;
        }
        Ok(addr)
    }

    /// Extract exactly the block [addr, addr + 2^order pages) from the free
    /// lists, splitting a containing block if needed.
    ///
    /// # Safety: addr is physically mapped and page-aligned.
    unsafe fn carve_block(&mut self, addr: u64, order: usize) {
        // Fast path: block already free at this exact order.
        if self.list_remove(addr, order) {
            return;
        }

        // Slow path: find the smallest enclosing free block and split down.
        // list_remove decrements free_pages by 1<<k for the container;
        // list_push increments for each spare → net -(1<<order) for the carved block.
        // The caller (buddy_alloc_at / buddy_remove_page) does NOT need to
        // adjust free_pages separately for carve_block's work.
        for k in (order + 1)..=MAX_ORDER {
            let k_bytes = (1u64 << k) * PAGE_SIZE;
            let container = addr & !(k_bytes - 1);
            if self.list_remove(container, k) {
                let mut current = container;
                let mut current_k = k;
                while current_k > order {
                    current_k -= 1;
                    let half_bytes = (1u64 << current_k) * PAGE_SIZE;
                    let spare = current + half_bytes;
                    if addr >= spare {
                        self.list_push(current, current_k);
                        current = spare;
                    } else {
                        self.list_push(spare, current_k);
                    }
                }
                return;
            }
        }
        // Not in any free list — already allocated or reserved.  Silently no-op.
    }

    /// Return block at `addr` / `order` to the buddy system, coalescing upward.
    ///
    /// # Safety: addr is physically mapped, page-aligned, not in any free list.
    unsafe fn buddy_free(&mut self, addr: u64, order: usize) {
        let mut current = addr;
        let mut current_k = order;

        // list_remove decrements free_pages for each merged buddy;
        // list_push at the end increments for the coalesced block.
        // Net = +1<<order (only the newly freed block's pages).
        while current_k < MAX_ORDER {
            let buddy = Self::buddy_of(current, current_k);
            if self.list_remove(buddy, current_k) {
                // Buddy was free — merge.
                current = current.min(buddy);
                current_k += 1;
            } else {
                break;
            }
        }

        self.list_push(current, current_k);
    }

    /// Decompose physical range [base, end) into natural buddy blocks and add
    /// each to the appropriate free list.
    unsafe fn buddy_add_range(&mut self, base: u64, end: u64) {
        let mut cur = base;
        while cur < end {
            let remaining_pages = (end - cur) / PAGE_SIZE;
            if remaining_pages == 0 {
                break;
            }
            let align_order = (cur.trailing_zeros() as usize)
                .saturating_sub(PAGE_SHIFT as usize)
                .min(MAX_ORDER);
            let size_order = (usize::BITS as usize - 1 - remaining_pages.leading_zeros() as usize)
                .min(MAX_ORDER);
            let order = align_order.min(size_order);
            // list_push now tracks free_pages automatically.
            self.list_push(cur, order);
            cur += (1u64 << order) * PAGE_SIZE;
        }
    }

    /// Remove a single order-0 page at `addr` from the buddy free lists,
    /// carving it out of a larger block if needed.
    unsafe fn buddy_remove_page(&mut self, addr: u64) {
        // carve_block now tracks free_pages via list_remove/push; no manual adj.
        self.carve_block(addr, 0);
    }

    // snapshot bookkeeping (E820 + type queries only)

    fn map_snapshot_add(&mut self, addr: u64, pages: u64, mem_type: MemoryType) {
        // Update existing overlapping free descriptor if found.
        for i in 0..self.map_count {
            if self.map[i].mem_type.is_free() && self.map[i].contains(addr) {
                self.map[i].mem_type = mem_type;
                self.map[i].physical_start = addr;
                self.map[i].number_of_pages = pages;
                return;
            }
        }
        // Append new entry.
        if self.map_count < MAX_MAP {
            self.map[self.map_count] = MemoryDescriptor {
                mem_type,
                physical_start: addr,
                virtual_start: 0,
                number_of_pages: pages,
                attribute: MemoryAttribute::WB,
            };
            self.map_count += 1;
        }
    }

    fn map_snapshot_remove(&mut self, addr: u64, pages: u64) {
        for i in 0..self.map_count {
            let d = &self.map[i];
            if d.physical_start == addr && d.number_of_pages == pages {
                self.map[i].mem_type = MemoryType::Conventional;
                return;
            }
        }
    }

    // diagnostics

    /// Walk every free-list chain and verify pointer integrity.
    /// Returns the number of corrupted pointers found.
    /// Called after `import_uefi_map` and before first allocation to catch
    /// scrub-poison or interrupt-race corruption early.
    pub fn validate_free_lists(&self) -> usize {
        let mut corrupt = 0usize;
        for order in 0..=MAX_ORDER {
            let mut cur = self.free_lists[order];
            let mut idx = 0u64;
            while !cur.is_null() {
                if !Self::is_canonical(cur) {
                    puts("[MEM] VALIDATE: corrupt ptr in free_lists[");
                    put_hex32(order as u32);
                    puts("] at node #");
                    put_hex32(idx as u32);
                    puts(": ptr=");
                    put_hex64(cur as u64);
                    puts("\n");
                    corrupt += 1;
                    break;
                }
                idx += 1;
                if idx > 0x800_000 {
                    // More than 8M nodes → infinite loop or circular chain.
                    puts("[MEM] VALIDATE: probable loop in free_lists[");
                    put_hex32(order as u32);
                    puts("]\n");
                    corrupt += 1;
                    break;
                }
                unsafe {
                    let next = (*cur).next;
                    if !next.is_null() && !Self::is_canonical(next) {
                        puts("[MEM] VALIDATE: corrupt next at node ");
                        put_hex64(cur as u64);
                        puts(" → ");
                        put_hex64(next as u64);
                        puts(" order=");
                        put_hex32(order as u32);
                        puts("\n");
                        corrupt += 1;
                        break;
                    }
                    cur = next;
                }
            }
        }
        if corrupt == 0 {
            puts("[MEM] free-list validation: OK\n");
        } else {
            puts("[MEM] free-list validation: ");
            put_hex32(corrupt as u32);
            puts(" corrupted chains!\n");
        }
        corrupt
    }

    /// Dump the UEFI memory map snapshot to serial for offline analysis.
    pub fn dump_map(&self) {
        puts("[MEM] ---- UEFI memory map (");
        put_hex32(self.map_count as u32);
        puts(" entries) ----\n");
        for i in 0..self.map_count {
            let d = &self.map[i];
            let ty = d.mem_type as u32;
            puts("  [");
            put_hex32(i as u32);
            puts("] type=");
            put_hex32(ty);
            puts(" phys=");
            put_hex64(d.physical_start);
            puts(" pages=");
            put_hex32(d.number_of_pages as u32);
            puts(" attr=");
            put_hex64(d.attribute.0);
            puts("\n");
        }
        puts("[MEM] ---- end map ----\n");
    }

    fn print_summary(&self) {
        let total_mb = (self.total_pages * PAGE_SIZE) >> 20;
        let free_mb = (self.free_pages * PAGE_SIZE) >> 20;
        puts("[MEM] buddy allocator ready: total=");
        put_hex32(total_mb as u32);
        puts("MB free=");
        put_hex32(free_mb as u32);
        puts("MB regions=");
        put_hex32(self.map_count as u32);
        puts("\n");
    }
}

// CR3 GUARD

/// Validate a CR3 candidate: page-aligned, non-zero, within physical address
/// space (< 2^52).
#[inline]
pub fn is_valid_cr3(cr3: u64) -> bool {
    cr3 != 0 && cr3 & 0xFFF == 0 && cr3 < (1u64 << 52)
}

/// RAII guard that switches to the kernel's CR3 on creation and restores
/// the previous CR3 on drop.  Ensures buddy allocator free-list traversals
/// see identity-mapped physical pages even when called from a user process
/// context.
///
/// If the kernel CR3 is not yet available (pre-scheduler init) or we're
/// already running under it, this is a no-op.
pub(crate) struct KernelCr3Guard {
    saved_cr3: u64,
    switched: bool,
}

impl KernelCr3Guard {
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn enter() -> Self {
        let kcr3 = crate::process::scheduler::get_kernel_cr3();
        if kcr3 == 0 {
            return Self {
                saved_cr3: 0,
                switched: false,
            };
        }
        if !is_valid_cr3(kcr3) {
            return Self {
                saved_cr3: 0,
                switched: false,
            };
        }
        let saved: u64;
        core::arch::asm!("mov {}, cr3", out(reg) saved, options(nostack, nomem));
        if saved == kcr3 {
            return Self {
                saved_cr3: saved,
                switched: false,
            };
        }
        core::arch::asm!("mov cr3, {}", in(reg) kcr3, options(nostack, nomem));
        Self {
            saved_cr3: saved,
            switched: true,
        }
    }

    #[inline]
    #[cfg(not(target_arch = "x86_64"))]
    pub unsafe fn enter() -> Self {
        Self {
            saved_cr3: 0,
            switched: false,
        }
    }
}

impl Drop for KernelCr3Guard {
    #[inline]
    fn drop(&mut self) {
        if self.switched {
            unsafe {
                #[cfg(target_arch = "x86_64")]
                core::arch::asm!("mov cr3, {}", in(reg) self.saved_cr3, options(nostack, nomem));
            }
        }
    }
}

// ORDER ARITHMETIC

/// Smallest order k such that 2ᵏ ≥ pages.
#[inline]
fn pages_to_order(pages: u64) -> usize {
    if pages <= 1 {
        return 0;
    }
    let p2 = pages.next_power_of_two();
    (p2.trailing_zeros() as usize).min(MAX_ORDER)
}

// GLOBAL REGISTRY

static mut GLOBAL_REGISTRY: MemoryRegistry = MemoryRegistry::new();
static mut REGISTRY_INITIALIZED: bool = false;

/// Parse and ingest the UEFI memory map into the global registry.
///
/// # Safety
/// Call exactly once, immediately after `ExitBootServices`, single-threaded.
/// `hw_holes`: sorted slice of page-aligned physical addresses that must be
/// excluded from the buddy (live page-table pages, GDT, IDT, etc.).
pub unsafe fn init_global_registry(
    map_ptr: *const u8,
    map_size: usize,
    descriptor_size: usize,
    descriptor_version: u32,
    exclude_base: u64,
    exclude_pages: u64,
    hw_holes: &[u64],
) {
    if REGISTRY_INITIALIZED {
        puts("[MEM] WARNING: registry already initialized!\n");
        return;
    }
    GLOBAL_REGISTRY.import_uefi_map(
        map_ptr,
        map_size,
        descriptor_size,
        descriptor_version,
        exclude_base,
        exclude_pages,
        hw_holes,
    );
    REGISTRY_INITIALIZED = true;
}

/// # Safety: bare-metal, single-threaded.
pub unsafe fn global_registry() -> &'static MemoryRegistry {
    &GLOBAL_REGISTRY
}
/// # Safety: bare-metal, single-threaded.
pub unsafe fn global_registry_mut() -> &'static mut MemoryRegistry {
    &mut GLOBAL_REGISTRY
}

pub fn is_registry_initialized() -> bool {
    unsafe { REGISTRY_INITIALIZED }
}

// LEGACY COMPATIBILITY SHIMS

/// Type alias — MemoryRegistry was previously called PhysicalMemoryMap in some callers.
pub type PhysicalMemoryMap = MemoryRegistry;

/// Type alias — MemoryDescriptor was previously called MemoryRegion.
pub type MemoryRegion = MemoryDescriptor;

/// Minimal bump allocator for early pre-registry use (rarely needed now).
pub struct PhysicalAllocator {
    current: u64,
    end: u64,
}

impl PhysicalAllocator {
    pub const fn new(base: u64, size: u64) -> Self {
        Self {
            current: base,
            end: base.wrapping_add(size),
        }
    }

    pub fn alloc_pages(&mut self, count: usize) -> Option<u64> {
        let aligned = (self.current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let size = (count as u64) * PAGE_SIZE;
        if aligned + size > self.end {
            return None;
        }
        self.current = aligned + size;
        Some(aligned)
    }

    pub fn alloc_bytes(&mut self, size: usize) -> Option<u64> {
        let aligned = (self.current + 15) & !15;
        let end = aligned + size as u64;
        if end > self.end {
            return None;
        }
        self.current = end;
        Some(aligned)
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.current)
    }
}

/// Parse a UEFI memory map and return a standalone (non-global) registry.
///
/// # Safety: Same as `import_uefi_map`.
pub unsafe fn parse_uefi_memory_map(
    map_ptr: *const u8,
    map_size: usize,
    desc_size: usize,
) -> MemoryRegistry {
    let mut r = MemoryRegistry::new();
    r.import_uefi_map(map_ptr, map_size, desc_size, 1, 0, 0, &[]);
    r
}

pub fn fallback_allocator() -> PhysicalAllocator {
    puts("[MEM] WARNING: using fallback allocator (16MB-32MB)\n");
    PhysicalAllocator::new(0x0100_0000, 16 * 1024 * 1024)
}
