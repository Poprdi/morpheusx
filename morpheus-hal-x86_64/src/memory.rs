//! Physical memory buddy allocator. Intrusive free-lists, XOR buddy math.
//! MAX_ORDER = 26 → 256 GiB max block. ~13 KiB BSS, no bitmap.

use crate::serial::{put_hex32, put_hex64, puts};

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;

/// Lets `KernelCr3Guard::enter` reach the kernel page table without a back-ref to
/// `process::scheduler`. Returns 0 before install.
pub type KernelCr3Fn = unsafe fn() -> u64;

static mut KERNEL_CR3_HOOK: Option<KernelCr3Fn> = None;

/// # Safety
/// Single-threaded; no concurrent `KernelCr3Guard::enter`.
pub unsafe fn set_kernel_cr3_hook(hook: KernelCr3Fn) {
    KERNEL_CR3_HOOK = Some(hook);
}

#[inline]
unsafe fn kernel_cr3_lookup() -> u64 {
    match KERNEL_CR3_HOOK {
        Some(h) => h(),
        None => 0,
    }
}

/// 26 → 256 GiB, 28 → 1 TiB, 30 → 4 TiB.
const MAX_ORDER: usize = 26;

/// UEFI snapshot slots; real hardware can post 400+.
const MAX_MAP: usize = 768;

/// UEFI EFI_MEMORY_TYPE + custom 0x8000_xxxx allocator tags.
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

    // 0x8000_xxxx range: safe from UEFI collisions.
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

    /// Excludes BootServices{Code,Data}: EDK2 leaves WP set, writing FreeNode → #PF.
    pub fn is_free(&self) -> bool {
        matches!(
            self,
            Self::Conventional | Self::LoaderCode | Self::LoaderData
        )
    }

    /// Alias of `is_free` for call-site readability.
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

    pub fn to_e820(self) -> E820Type {
        match self {
            Self::Conventional
            | Self::LoaderCode
            | Self::LoaderData
            // Post-EBS, BootServices{Code,Data} are usable RAM in E820.
            | Self::BootServicesCode
            | Self::BootServicesData
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

/// UEFI EFI_MEMORY_* bits verbatim.
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

/// E820 type tags (Linux handoff).
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

/// UEFI EFI_MEMORY_DESCRIPTOR layout.
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

/// Mirrors UEFI AllocateType.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateType {
    AnyPages,
    /// End ≤ limit (DMA < 4 GiB).
    MaxAddress(u64),
    /// Exact, page-aligned.
    Address(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    OutOfResources,
    InvalidParameter,
    NotFound,
    BufferTooSmall,
    AlreadyAllocated,
}

/// 16-byte intrusive free-list node stamped into each free page.
#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

/// Buddy allocator.
pub struct MemoryRegistry {
    free_lists: [*mut FreeNode; MAX_ORDER + 1],
    free_at_order: [u64; MAX_ORDER + 1],
    total_pages: u64,
    free_pages: u64,
    allocated_pages: u64,
    /// Monotonic; callers detect map changes via this.
    map_key: u64,

    /// Snapshot for type queries + E820 only; not for allocation.
    map: [MemoryDescriptor; MAX_MAP],
    map_count: usize,

    /// PE-image exclusion zone preserved for the reclaim pass.
    excl_start: u64,
    excl_end: u64,
}

// SAFETY: bare-metal, identity-mapped, no SMP races at this layer.
unsafe impl Send for MemoryRegistry {}
unsafe impl Sync for MemoryRegistry {}

impl Default for MemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryRegistry {
    /// `const` so the static lives in BSS.
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

    /// Parse UEFI memory map into the buddy. `exclude_base/exclude_pages` punch out
    /// the PE image range so the allocator doesn't scribble FreeNode over our `.bss`.
    /// `hw_holes` is a sorted slice of page-aligned phys addrs (live PT pages, GDT,
    /// IDT) that must never enter the buddy — FreeNode over any of those = #GP/#PF.
    ///
    /// # Safety
    /// Once, single-threaded, pre-first-alloc. `map_ptr` describes a valid UEFI
    /// memory map of `map_size` bytes with `descriptor_size` stride.
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
                // Conventional/LoaderCode/LoaderData only — BootServices still WP.
                // Skip first 1 MiB (BIOS land).
                let region_end = phys + pages * PAGE_SIZE;
                let start = if phys < 0x10_0000 { 0x10_0000 } else { phys };
                if start < region_end {
                    // Clip PE image first, then punch hw_holes so FreeNode never
                    // lands in a live CPU structure.
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

    /// Punch a hole in the buddy. Protect PT pages, stacks, etc.
    ///
    /// # Safety
    /// `base`/`pages` must describe a real physical range that is genuinely
    /// reserved; removing live pages from the allocator or reserving the same
    /// range twice corrupts the buddy state. Init/single-threaded use only.
    pub unsafe fn reserve_range(&mut self, base: u64, pages: u64) {
        let mut addr = base & !(PAGE_SIZE - 1);
        let end = addr + pages * PAGE_SIZE;
        while addr < end {
            self.buddy_remove_page(addr);
            addr += PAGE_SIZE;
        }
    }

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

        // FreeNodes live at identity-mapped phys; user CR3 may remap those VAs.
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

    /// Decomposes into aligned buddy blocks; coalesces automatically.
    /// `free_pages` counter is maintained entirely by list_push/remove/pop.
    pub fn free_pages(&mut self, addr: u64, pages: u64) -> Result<(), MemoryError> {
        if addr & (PAGE_SIZE - 1) != 0 || pages == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        self.map_snapshot_remove(addr, pages);

        // Free-list mutations require kernel CR3 mapping the buddy phys range.
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

    /// Page-rounded byte alloc.
    pub fn allocate_pool(&mut self, size: usize) -> Result<u64, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidParameter);
        }
        let pages = (size as u64).div_ceil(PAGE_SIZE);
        self.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, pages)
    }

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

    /// Reclaim UEFI BootServices{Code,Data} into the buddy. Follow with
    /// `reserve_page_table_pages()` to re-lock any PT pages that were in
    /// BootServices address space. Applies the same low-1 MiB skip and PE-image
    /// exclusion as the initial import. `cpu_excl` must be sorted ascending and
    /// list live PT/GDT/IDT pages — FreeNode over any of those = #GP/#PF.
    ///
    /// # Safety
    /// Single-threaded, post-`import_uefi_map`, after every late-boot subsystem
    /// has stopped touching BootServices.
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

            // Skip low 1 MiB (BIOS land, VGA, ROM shadows).
            let start = if phys < 0x10_0000 { 0x10_0000 } else { phys };
            if start >= region_end {
                continue;
            }

            // PE-exclusion first → up to 2 sub-ranges; hole-punch each.
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

            for &(s_start, s_end) in subs.iter().take(nsub) {
                reclaimed_pages += self.add_range_punching_holes(s_start, s_end, cpu_excl);
            }
        }

        let reclaimed_mb = (reclaimed_pages * PAGE_SIZE) >> 20;
        let _ = reclaimed_mb;
        crate::serial::log_info("MEM", 762, "reclaimed boot services memory");
    }

    /// Add `[start, end)`, skipping `holes`. `holes` must be sorted ascending.
    /// Returns pages added.
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
            if cur < hole {
                let pages = (hole - cur) / PAGE_SIZE;
                self.buddy_add_range(cur, hole);
                added += pages;
            }
            cur = hole + PAGE_SIZE;
        }

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

    /// Legacy bump shim; always 0.
    pub fn bump_remaining(&self) -> u64 {
        0
    }

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
                    cur = core::ptr::read_volatile(&(*cur).next);
                }
            }
        }
        None
    }

    /// Snapshot lookup.
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

    /// Phys range ≤ 4 GiB.
    pub fn alloc_dma_pages(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(
            AllocateType::MaxAddress(0xFFFF_FFFF),
            MemoryType::AllocatedDma,
            pages,
        )
    }

    /// Page-rounded; phys range ≤ 4 GiB.
    pub fn alloc_dma_bytes(&mut self, size: usize) -> Result<u64, MemoryError> {
        let pages = (size as u64).div_ceil(PAGE_SIZE);
        self.alloc_dma_pages(pages)
    }

    pub fn alloc_stack(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedStack, pages)
    }

    /// Validates a free-list pointer. Buddy FreeNodes always live in
    /// identity-mapped phys RAM (lower half), so we reject:
    ///   - non-canonical addresses (bits 47..63 not sign-extended)
    ///   - canonical kernel-half addresses (top17 == 0x1FFFF; e.g. !0,
    ///     OVMF/firmware poison, real-HW UEFI residue that's canonical but
    ///     always unmapped from the kernel PT — observed crashing the
    ///     post-reclaim validate walk on Intel silicon)
    ///
    /// Null returns true; callers null-check separately.
    #[inline]
    fn is_canonical(ptr: *mut FreeNode) -> bool {
        if ptr.is_null() {
            return true;
        }
        let addr = ptr as u64;
        // Lower-half canonical only.
        addr >> 47 == 0
    }

    /// # Safety
    /// `addr` 4 KiB aligned and physically mapped.
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
            core::ptr::write_volatile(&mut (*node).next, core::ptr::null_mut());
            core::ptr::write_volatile(&mut (*node).prev, core::ptr::null_mut());
            self.free_at_order[order] = 1;
            self.free_pages += 1u64 << order;
            return;
        }
        core::ptr::write_volatile(&mut (*node).next, old_head);
        core::ptr::write_volatile(&mut (*node).prev, core::ptr::null_mut());
        if !old_head.is_null() {
            core::ptr::write_volatile(&mut (*old_head).prev, node);
        }
        self.free_lists[order] = node;
        self.free_at_order[order] += 1;
        self.free_pages += 1u64 << order;
    }

    /// Returns true if found and removed.
    unsafe fn list_remove(&mut self, addr: u64, order: usize) -> bool {
        let target = addr as *mut FreeNode;
        let mut cur = self.free_lists[order];
        while !cur.is_null() {
            if cur == target {
                let prev = core::ptr::read_volatile(&(*cur).prev);
                let next = core::ptr::read_volatile(&(*cur).next);
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
                    // Sever the chain; chain state unknown so leave free_pages alone.
                    self.free_lists[order] = core::ptr::null_mut();
                    self.free_at_order[order] = 0;
                    return true;
                }
                if !prev.is_null() {
                    core::ptr::write_volatile(&mut (*prev).next, next);
                } else {
                    self.free_lists[order] = next;
                }
                if !next.is_null() {
                    core::ptr::write_volatile(&mut (*next).prev, prev);
                }
                self.free_at_order[order] -= 1;
                self.free_pages = self.free_pages.saturating_sub(1u64 << order);
                return true;
            }
            let next = core::ptr::read_volatile(&(*cur).next);
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
                // Terminate corrupted chain.
                core::ptr::write_volatile(&mut (*cur).next, core::ptr::null_mut());
                break;
            }
            cur = next;
        }
        false
    }

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
            // Chain state unknown; leave free_pages alone.
            return None;
        }
        let next = core::ptr::read_volatile(&(*head).next);
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
            // Returning one block — deduct only that.
            self.free_pages = self.free_pages.saturating_sub(1u64 << order);
            return Some(head as u64);
        }
        self.free_lists[order] = next;
        if !next.is_null() {
            core::ptr::write_volatile(&mut (*next).prev, core::ptr::null_mut());
        }
        self.free_at_order[order] -= 1;
        self.free_pages = self.free_pages.saturating_sub(1u64 << order);
        Some(head as u64)
    }

    #[inline(always)]
    fn buddy_of(addr: u64, order: usize) -> u64 {
        addr ^ ((1u64 << order) * PAGE_SIZE)
    }

    /// Allocate 2^order pages anywhere.
    fn buddy_alloc(&mut self, order: usize) -> Result<u64, MemoryError> {
        let top = (order..=MAX_ORDER)
            .find(|&k| !self.free_lists[k].is_null())
            .ok_or(MemoryError::OutOfResources)?;

        let addr = unsafe { self.list_pop(top).unwrap() };

        // Split top→order; list_pop deducted 1<<top, each list_push adds back the spare,
        // net change = -(1<<order). No manual free_pages adjust.
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

    /// Allocate 2^order pages entirely ≤ `limit`.
    fn buddy_alloc_below(&mut self, order: usize, limit: u64) -> Result<u64, MemoryError> {
        let block_bytes = (1u64 << order) * PAGE_SIZE;

        // Top-down: fewer splits.
        for k in (order..=MAX_ORDER).rev() {
            let k_bytes = (1u64 << k) * PAGE_SIZE;
            let mut cur = self.free_lists[k];
            while !cur.is_null() {
                let base = cur as u64;
                let base_end = base.saturating_add(k_bytes);
                if base.saturating_add(block_bytes) <= limit.saturating_add(1)
                    && base_end <= limit.saturating_add(1)
                {
                    // Net change = -(1<<order); see `buddy_alloc`.
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
                    cur = core::ptr::read_volatile(&(*cur).next);
                }
            }
        }
        Err(MemoryError::OutOfResources)
    }

    /// Allocate exactly `pages` at fixed phys `addr`.
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
            // SAFETY: identity-mapped phys range.
            unsafe {
                self.carve_block(base, order);
            }
            self.allocated_pages += 1u64 << order;
            base += (1u64 << order) * PAGE_SIZE;
        }
        Ok(addr)
    }

    /// Extract `[addr, addr + 2^order pages)` from the free lists; splits a
    /// containing block as needed. Net free_pages change = -(1<<order).
    ///
    /// # Safety
    /// `addr` page-aligned and physically mapped.
    unsafe fn carve_block(&mut self, addr: u64, order: usize) {
        if self.list_remove(addr, order) {
            return;
        }

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
        // Already allocated or reserved; no-op.
    }

    /// Return `addr`/`order` to the buddy, coalescing upward. Net +1<<order.
    ///
    /// # Safety
    /// `addr` physically mapped, page-aligned, not in any free list.
    unsafe fn buddy_free(&mut self, addr: u64, order: usize) {
        let mut current = addr;
        let mut current_k = order;

        while current_k < MAX_ORDER {
            let buddy = Self::buddy_of(current, current_k);
            if self.list_remove(buddy, current_k) {
                current = current.min(buddy);
                current_k += 1;
            } else {
                break;
            }
        }

        self.list_push(current, current_k);
    }

    /// Decompose `[base, end)` into natural buddy blocks and push each.
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
            // Zero header first to destroy any OVMF 0xAFAFAFAF poison so
            // subsequent list_remove walks stay clean.
            let node_ptr = cur as *mut u64;
            core::ptr::write_volatile(node_ptr, 0u64); // next
            core::ptr::write_volatile(node_ptr.add(1), 0u64); // prev
            self.list_push(cur, order);
            cur += (1u64 << order) * PAGE_SIZE;
        }
    }

    /// Carve a single order-0 page out, splitting upward as needed.
    unsafe fn buddy_remove_page(&mut self, addr: u64) {
        self.carve_block(addr, 0);
    }

    // Snapshot bookkeeping (E820 + type queries only).

    fn map_snapshot_add(&mut self, addr: u64, pages: u64, mem_type: MemoryType) {
        // Update an overlapping free descriptor if present.
        for i in 0..self.map_count {
            if self.map[i].mem_type.is_free() && self.map[i].contains(addr) {
                self.map[i].mem_type = mem_type;
                self.map[i].physical_start = addr;
                self.map[i].number_of_pages = pages;
                return;
            }
        }
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

    /// Walks every free-list chain checking canonicality; run between
    /// `import_uefi_map` and first alloc to catch scrub-poison / IRQ-race
    /// corruption early. Returns count of corrupted pointers.
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
                    // >8M nodes ⇒ loop / circular chain.
                    puts("[MEM] VALIDATE: probable loop in free_lists[");
                    put_hex32(order as u32);
                    puts("]\n");
                    corrupt += 1;
                    break;
                }
                unsafe {
                    let next = core::ptr::read_volatile(&(*cur).next);
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
            crate::serial::log_ok("MEM", 760, "free-list validation passed");
        } else {
            crate::serial::log_warn("MEM", 761, "free-list validation found corruption");
        }
        corrupt
    }

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
        let _ = (total_mb, free_mb);
        crate::serial::log_ok("MEM", 763, "buddy allocator ready");
    }
}

/// Page-aligned, non-zero, phys-addr space (< 2^52).
#[inline]
pub fn is_valid_cr3(cr3: u64) -> bool {
    cr3 != 0 && cr3 & 0xFFF == 0 && cr3 < (1u64 << 52)
}

/// Switches to kernel CR3 on `enter`, restores on drop. Ensures buddy free-list
/// walks see identity-mapped phys even from a user-CR3 context. No-op pre-scheduler
/// init or when already on kernel CR3.
pub struct KernelCr3Guard {
    saved_cr3: u64,
    switched: bool,
    interrupts_were_enabled: bool,
}

impl KernelCr3Guard {
    /// # Safety
    /// Switches the active CR3 to the kernel page tables and toggles interrupts.
    /// The kernel CR3 must be established and the guard must be dropped on the
    /// same core before any user-CR3-dependent code runs.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn enter() -> Self {
        let kcr3 = kernel_cr3_lookup();
        if kcr3 == 0 || !is_valid_cr3(kcr3) {
            return Self {
                saved_cr3: 0,
                switched: false,
                interrupts_were_enabled: false,
            };
        }
        let interrupts_were_enabled = crate::intr::interrupts_enabled();
        crate::intr::disable_interrupts();
        let saved: u64;
        core::arch::asm!("mov {}, cr3", out(reg) saved, options(nostack, nomem));
        if saved == kcr3 {
            if interrupts_were_enabled {
                crate::intr::enable_interrupts();
            }
            return Self {
                saved_cr3: saved,
                switched: false,
                interrupts_were_enabled: false,
            };
        }
        core::arch::asm!("mov cr3, {}", in(reg) kcr3, options(nostack, nomem));
        Self {
            saved_cr3: saved,
            switched: true,
            interrupts_were_enabled,
        }
    }

    #[inline]
    #[cfg(not(target_arch = "x86_64"))]
    pub unsafe fn enter() -> Self {
        Self {
            saved_cr3: 0,
            switched: false,
            interrupts_were_enabled: false,
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
                if self.interrupts_were_enabled {
                    crate::intr::enable_interrupts();
                }
            }
        }
    }
}
/// Smallest k with 2^k ≥ pages.
#[inline]
fn pages_to_order(pages: u64) -> usize {
    if pages <= 1 {
        return 0;
    }
    let p2 = pages.next_power_of_two();
    (p2.trailing_zeros() as usize).min(MAX_ORDER)
}

use crate::sync::{SpinLock, SpinLockGuard};

static GLOBAL_REGISTRY: SpinLock<MemoryRegistry> = SpinLock::new(MemoryRegistry::new());
static REGISTRY_INITIALIZED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// # Safety
/// Once, single-threaded, immediately after `ExitBootServices`. `hw_holes`
/// sorted ascending — live PT pages, GDT, IDT, etc.
pub unsafe fn init_global_registry(
    map_ptr: *const u8,
    map_size: usize,
    descriptor_size: usize,
    descriptor_version: u32,
    exclude_base: u64,
    exclude_pages: u64,
    hw_holes: &[u64],
) {
    if REGISTRY_INITIALIZED.load(core::sync::atomic::Ordering::Relaxed) {
        puts("[MEM] WARNING: registry already initialized!\n");
        return;
    }
    GLOBAL_REGISTRY.lock().import_uefi_map(
        map_ptr,
        map_size,
        descriptor_size,
        descriptor_version,
        exclude_base,
        exclude_pages,
        hw_holes,
    );
    REGISTRY_INITIALIZED.store(true, core::sync::atomic::Ordering::Release);
}

/// # Safety
/// Lock held for the guard's lifetime.
pub unsafe fn global_registry() -> SpinLockGuard<'static, MemoryRegistry> {
    GLOBAL_REGISTRY.lock()
}
/// # Safety
/// Lock held for the guard's lifetime.
pub unsafe fn global_registry_mut() -> SpinLockGuard<'static, MemoryRegistry> {
    GLOBAL_REGISTRY.lock()
}

pub fn is_registry_initialized() -> bool {
    REGISTRY_INITIALIZED.load(core::sync::atomic::Ordering::Acquire)
}

// Legacy aliases.
pub type PhysicalMemoryMap = MemoryRegistry;
pub type MemoryRegion = MemoryDescriptor;

/// Bump allocator for pre-registry boot.
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

/// Standalone (non-global) registry.
///
/// # Safety
/// Same as `import_uefi_map`.
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
    PhysicalAllocator::new(0x0100_0000, 16 * 1024 * 1024)
}
