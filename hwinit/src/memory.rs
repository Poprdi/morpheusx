//! Physical Memory — Binary Buddy Allocator
//!
//! A buddy system over the full physical address space.
//! Allocation and freeing are O(log₂ N) where N = total pages.
//! No bitmap, no descriptor table that overflows, no fixed region limit.
//!
//! # The Algorithm
//!
//! Every free block has size 2ᵏ pages (k = "order", 0 ≤ k ≤ MAX_ORDER).
//! The buddy of block at address A with order k is at A ^ (1 << (k + PAGE_SHIFT)).
//!
//! Alloc(k): take from free_lists[k]; if empty, split a block from order k+1,
//!            push the spare half onto free_lists[k], return the other half.
//!
//! Free(A, k): compute buddy = A ^ (1 << (k+12)); if buddy is in free_lists[k],
//!             remove it, merge into a block of order k+1 and repeat.
//!             Otherwise push A onto free_lists[k].
//!
//! # Capacity
//!
//!   MAX_ORDER = 26 → max contiguous block = 2²⁶ × 4 KiB = 256 GiB
//!   MAX_ORDER = 28 → 1 TiB   (change one constant — nothing else)
//!   MAX_ORDER = 30 → 4 TiB
//!
//! # Metadata
//!
//! Free-list nodes live INSIDE the free pages themselves (intrusive list).
//! The allocator struct is ~13 KiB in BSS.  There is no per-page array.
//!
//! # Public API
//!
//! Identical to the previous MemoryRegistry so all callers compile unchanged.

use crate::serial::{put_hex32, puts};

// ═══════════════════════════════════════════════════════════════════════════
// FUNDAMENTAL CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

pub const PAGE_SIZE:  u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;

/// Maximum buddy order.  2^MAX_ORDER × 4 KiB = maximum single allocation.
///
///   26 → 256 GiB    28 → 1 TiB    30 → 4 TiB
///
/// Only this constant needs changing to support a larger address space.
const MAX_ORDER: usize = 26;

/// Maximum entries in the UEFI-snapshot map (for type queries and E820 export).
/// Controls only a fixed backup store; allocation is free from this limit.
const MAX_MAP: usize = 384;

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY TYPES  (public — callers bind to these names)
// ═══════════════════════════════════════════════════════════════════════════

/// Memory type — mirrors UEFI EFI_MEMORY_TYPE extended with our own tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryType {
    Reserved            = 0,
    LoaderCode          = 1,
    LoaderData          = 2,
    BootServicesCode    = 3,
    BootServicesData    = 4,
    RuntimeServicesCode = 5,
    RuntimeServicesData = 6,
    Conventional        = 7,
    Unusable            = 8,
    AcpiReclaim         = 9,
    AcpiNvs             = 10,
    Mmio                = 11,
    MmioPortSpace       = 12,
    PalCode             = 13,
    Persistent          = 14,

    // Our custom allocator tags (high range, never overlap UEFI values).
    AllocatedDma        = 0x8000_0001,
    AllocatedStack      = 0x8000_0002,
    AllocatedPageTable  = 0x8000_0003,
    AllocatedHeap       = 0x8000_0004,
    Allocated           = 0x8000_0000,
}

impl MemoryType {
    pub fn from_uefi_raw(value: u32) -> Self {
        match value {
            0  => Self::Reserved,
            1  => Self::LoaderCode,
            2  => Self::LoaderData,
            3  => Self::BootServicesCode,
            4  => Self::BootServicesData,
            5  => Self::RuntimeServicesCode,
            6  => Self::RuntimeServicesData,
            7  => Self::Conventional,
            8  => Self::Unusable,
            9  => Self::AcpiReclaim,
            10 => Self::AcpiNvs,
            11 => Self::Mmio,
            12 => Self::MmioPortSpace,
            13 => Self::PalCode,
            14 => Self::Persistent,
            _  => Self::Reserved,
        }
    }

    /// True for memory that is free to use immediately after boot.
    ///
    /// Note: `BootServicesCode` and `BootServicesData` are semantically free
    /// after `ExitBootServices`, but EDK2 maps them with write-protection bits
    /// still set in the UEFI page tables.  Writing an intrusive free-list node
    /// into those pages before we own the page tables causes a #PF(W=1,P=1).
    /// We therefore exclude them here — they are tracked in the snapshot map
    /// for E820 accuracy but never given to the buddy system.
    pub fn is_free(&self) -> bool {
        matches!(
            self,
            Self::Conventional
                | Self::LoaderCode
                | Self::LoaderData
        )
    }

    /// True for memory that is free AND guaranteed writable right now
    /// (under UEFI's page tables — used for initial buddy population).
    /// Identical to `is_free()` by design; kept as a named alias so the
    /// invariant is explicit at the call site.
    pub fn is_immediately_writable(&self) -> bool { self.is_free() }

    pub fn is_reclaimable(&self) -> bool { matches!(self, Self::AcpiReclaim) }

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

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY ATTRIBUTES  (mirrors UEFI)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAttribute(pub u64);

impl MemoryAttribute {
    pub const UC:      Self = Self(0x0000_0000_0000_0001);
    pub const WC:      Self = Self(0x0000_0000_0000_0002);
    pub const WT:      Self = Self(0x0000_0000_0000_0004);
    pub const WB:      Self = Self(0x0000_0000_0000_0008);
    pub const UCE:     Self = Self(0x0000_0000_0000_0010);
    pub const WP:      Self = Self(0x0000_0000_0000_1000);
    pub const RP:      Self = Self(0x0000_0000_0000_2000);
    pub const XP:      Self = Self(0x0000_0000_0000_4000);
    pub const NV:      Self = Self(0x0000_0000_0000_8000);
    pub const MORE_RELIABLE: Self = Self(0x0000_0000_0001_0000);
    pub const RO:      Self = Self(0x0000_0000_0002_0000);
    pub const SP:      Self = Self(0x0000_0000_0004_0000);
    pub const RUNTIME: Self = Self(0x8000_0000_0000_0000);

    pub const fn empty()             -> Self { Self(0) }
    pub const fn contains(self, o: Self) -> bool { (self.0 & o.0) == o.0 }
    pub const fn union(self, o: Self) -> Self { Self(self.0 | o.0) }
}

// ═══════════════════════════════════════════════════════════════════════════
// E820  (for Linux handoff)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum E820Type {
    Ram      = 1,
    Reserved = 2,
    Acpi     = 3,
    Nvs      = 4,
    Unusable = 5,
    Disabled = 6,
    Pmem     = 7,
    Undefined = 8,
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct E820Entry {
    pub addr:       u64,
    pub size:       u64,
    pub entry_type: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY DESCRIPTOR  (mirrors UEFI EFI_MEMORY_DESCRIPTOR)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub mem_type:        MemoryType,
    pub physical_start:  u64,
    pub virtual_start:   u64,
    pub number_of_pages: u64,
    pub attribute:       MemoryAttribute,
}

impl MemoryDescriptor {
    pub const fn empty() -> Self {
        Self {
            mem_type:        MemoryType::Reserved,
            physical_start:  0,
            virtual_start:   0,
            number_of_pages: 0,
            attribute:       MemoryAttribute::empty(),
        }
    }
    pub const fn physical_end(&self) -> u64 { self.physical_start + self.number_of_pages * PAGE_SIZE }
    pub const fn size(&self)         -> u64 { self.number_of_pages * PAGE_SIZE }
    pub const fn contains(&self, addr: u64) -> bool {
        addr >= self.physical_start && addr < self.physical_end()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ALLOCATION TYPE  (mirrors UEFI EFI_ALLOCATE_TYPE)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateType {
    /// Any free pages.
    AnyPages,
    /// Highest address whose end ≤ specified limit (needed for DMA < 4 GB).
    MaxAddress(u64),
    /// Exactly this physical address (page-aligned).
    Address(u64),
}

// ═══════════════════════════════════════════════════════════════════════════
// ERROR TYPE
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    OutOfResources,
    InvalidParameter,
    NotFound,
    BufferTooSmall,
    AlreadyAllocated,
}

// ═══════════════════════════════════════════════════════════════════════════
// BUDDY FREE-LIST NODE  — intrusive, lives inside each free block
// ═══════════════════════════════════════════════════════════════════════════

/// Written at offset 0 of every free physical block.
///
/// A free block of order k spans 2ᵏ × 4 KiB ≥ 4 KiB, which is always enough
/// room to embed these 16 bytes.
#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

// ═══════════════════════════════════════════════════════════════════════════
// BUDDY ALLOCATOR  (type alias kept as MemoryRegistry for ABI compat)
// ═══════════════════════════════════════════════════════════════════════════

/// Physical memory allocator — buddy system over the full address space.
///
/// Rename-proof: the public name `MemoryRegistry` is stable; internals are free
/// to evolve.  MAX_ORDER is the only knob needed to extend address range.
pub struct MemoryRegistry {
    /// free_lists[k] = head of doubly-linked free list for order k.
    /// null = empty.  Nodes live INSIDE the free pages (intrusive).
    free_lists: [*mut FreeNode; MAX_ORDER + 1],

    /// Pages free at each order (for diagnostics / sysinfo).
    free_at_order: [u64; MAX_ORDER + 1],

    /// Aggregate statistics.
    total_pages:     u64,
    free_pages:      u64,
    allocated_pages: u64,

    /// Monotonically increasing — callers can detect map changes.
    map_key: u64,

    /// UEFI memory map snapshot.
    /// Used ONLY for `memory_type_at()` and E820 export.
    /// Allocation state is owned entirely by the buddy lists above.
    map:       [MemoryDescriptor; MAX_MAP],
    map_count: usize,
}

// SAFETY: single-threaded bare-metal; raw pointers into identity-mapped RAM.
unsafe impl Send for MemoryRegistry {}
unsafe impl Sync for MemoryRegistry {}

impl MemoryRegistry {
    /// Create a zeroed, empty registry.  `const` so it fits in BSS.
    pub const fn new() -> Self {
        Self {
            free_lists:      [core::ptr::null_mut(); MAX_ORDER + 1],
            free_at_order:   [0; MAX_ORDER + 1],
            total_pages:     0,
            free_pages:      0,
            allocated_pages: 0,
            map_key:         0,
            map:             [MemoryDescriptor::empty(); MAX_MAP],
            map_count:       0,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // INITIALISATION  (called once, immediately after ExitBootServices)
    // ─────────────────────────────────────────────────────────────────────

    /// Parse the UEFI memory map and populate the buddy free lists.
    ///
    /// # Safety
    /// `map_ptr` must point to a valid UEFI memory map of `map_size` bytes
    /// with entries spaced `descriptor_size` bytes apart.
    /// Must be called exactly once, single-threaded, before any allocation.
    pub unsafe fn import_uefi_map(
        &mut self,
        map_ptr: *const u8,
        map_size: usize,
        descriptor_size: usize,
        _descriptor_version: u32,
    ) {
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
            let raw_type = *(ptr        as *const u32);
            let phys     = *(ptr.add(8) as *const u64);
            let virt     = *(ptr.add(16) as *const u64);
            let pages    = *(ptr.add(24) as *const u64);
            let attr     = *(ptr.add(32) as *const u64);

            if pages == 0 { continue; }

            let mem_type = MemoryType::from_uefi_raw(raw_type);

            // Snapshot into map for type queries / E820.
            if self.map_count < MAX_MAP {
                self.map[self.map_count] = MemoryDescriptor {
                    mem_type,
                    physical_start:  phys,
                    virtual_start:   virt,
                    number_of_pages: pages,
                    attribute:       MemoryAttribute(attr),
                };
                self.map_count += 1;
            }

            self.total_pages += pages;

            if mem_type.is_immediately_writable() {
                // Add this region to the buddy system.
                //
                // Only EfiConventional / LoaderCode / LoaderData are safe to
                // write into RIGHT NOW — UEFI's page tables still mark
                // BootServicesCode as execute-only and BootServicesData pages
                // may be write-protected.  Touching them here causes #PF W=1.
                //
                // Skip the first 1 MiB (legacy BIOS area — never safe to claim).
                let region_end = phys + pages * PAGE_SIZE;
                let start = if phys < 0x10_0000 { 0x10_0000 } else { phys };
                if start < region_end {
                    self.buddy_add_range(start, region_end);
                }
            }
        }

        self.map_key = 1;
        self.print_summary();
    }

    /// Punch a hole: remove pages [base, base + pages * PAGE_SIZE) from the
    /// buddy free lists.  Used immediately after import to protect live
    /// page tables, stacks, and other pre-allocated regions.
    ///
    /// # Safety
    /// `base` must be page-aligned.  Pages must currently be in the free lists.
    pub unsafe fn reserve_range(&mut self, base: u64, pages: u64) {
        let mut addr = base & !(PAGE_SIZE - 1);
        let end = addr + pages * PAGE_SIZE;
        while addr < end {
            self.buddy_remove_page(addr);
            addr += PAGE_SIZE;
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // PUBLIC ALLOCATION API  (same signature as previous MemoryRegistry)
    // ─────────────────────────────────────────────────────────────────────

    /// Allocate `pages` contiguous physical pages.
    ///
    /// Mirrors UEFI `AllocatePages()`.
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

        let addr = match alloc_type {
            AllocateType::AnyPages          => self.buddy_alloc(order)?,
            AllocateType::MaxAddress(limit) => self.buddy_alloc_below(order, limit)?,
            AllocateType::Address(want)     => self.buddy_alloc_at(want, pages)?,
        };

        self.map_snapshot_add(addr, pages, mem_type);
        self.map_key += 1;
        Ok(addr)
    }

    /// Free `pages` physical pages starting at `addr`.
    ///
    /// Mirrors UEFI `FreePages()`.  Decomposes the range into naturally-aligned
    /// buddy blocks and releases each one, triggering coalescing automatically.
    pub fn free_pages(&mut self, addr: u64, pages: u64) -> Result<(), MemoryError> {
        if addr & (PAGE_SIZE - 1) != 0 || pages == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        self.map_snapshot_remove(addr, pages);

        // Decompose [addr, addr+pages*PAGE_SIZE) into naturally-aligned
        // power-of-two blocks and return each to the buddy system.
        let mut base = addr;
        let end = addr + pages * PAGE_SIZE;
        while base < end {
            let remaining_pages = (end - base) / PAGE_SIZE;
            let align_order = (base.trailing_zeros() as usize).saturating_sub(PAGE_SHIFT as usize);
            let size_order  = usize::BITS as usize - 1 - remaining_pages.leading_zeros() as usize;
            let order = align_order.min(size_order).min(MAX_ORDER);
            // SAFETY: addr came from allocate_pages; identity-mapped.
            unsafe { self.buddy_free(base, order); }
            base += (1u64 << order) * PAGE_SIZE;
        }

        self.map_key += 1;
        Ok(())
    }

    /// Byte-granular allocation — returns a page-aligned block large enough.
    ///
    /// The previous bump-pool is gone; callers receive whole pages.  This is
    /// always correct because callers only write within their requested size.
    pub fn allocate_pool(&mut self, size: usize) -> Result<u64, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidParameter);
        }
        let pages = (size as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        self.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, pages)
    }

    // ─────────────────────────────────────────────────────────────────────
    // QUERY API  (same as previous MemoryRegistry)
    // ─────────────────────────────────────────────────────────────────────

    pub fn get_memory_map(&self) -> (u64, usize) { (self.map_key, self.map_count) }
    pub fn get_map_key(&self)    -> u64 { self.map_key }

    pub fn get_descriptor(&self, index: usize) -> Option<&MemoryDescriptor> {
        if index < self.map_count { Some(&self.map[index]) } else { None }
    }

    pub fn total_memory(&self)     -> u64 { self.total_pages * PAGE_SIZE }
    pub fn free_memory(&self)      -> u64 { self.free_pages.saturating_mul(PAGE_SIZE) }
    pub fn allocated_memory(&self) -> u64 { self.allocated_pages.saturating_mul(PAGE_SIZE) }

    /// Legacy: returned remaining bump space.  Always 0 now (no bump).
    pub fn bump_remaining(&self) -> u64 { 0 }

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
                unsafe { cur = (*cur).next; }
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
        for i in 0..n {
            let d = &self.map[i];
            buffer[i] = E820Entry {
                addr:       d.physical_start,
                size:       d.size(),
                entry_type: d.mem_type.to_e820() as u32,
            };
        }
        n
    }

    pub fn e820_count(&self) -> usize { self.map_count }

    // ─────────────────────────────────────────────────────────────────────
    // CONVENIENCE ALLOCATORS
    // ─────────────────────────────────────────────────────────────────────

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
        let pages = (size as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        self.alloc_dma_pages(pages)
    }

    /// Kernel stack pages (any address).
    pub fn alloc_stack(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(AllocateType::AnyPages, MemoryType::AllocatedStack, pages)
    }

    // ─────────────────────────────────────────────────────────────────────
    // BUDDY CORE — PRIVATE
    // ─────────────────────────────────────────────────────────────────────

    /// Push a block at `addr` onto free_lists[order].
    ///
    /// # Safety: addr must be 4-KiB aligned and physically mapped.
    unsafe fn list_push(&mut self, addr: u64, order: usize) {
        let node = addr as *mut FreeNode;
        let old_head = self.free_lists[order];
        (*node).next = old_head;
        (*node).prev = core::ptr::null_mut();
        if !old_head.is_null() {
            (*old_head).prev = node;
        }
        self.free_lists[order] = node;
        self.free_at_order[order] += 1;
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
                if !prev.is_null() { (*prev).next = next; } else { self.free_lists[order] = next; }
                if !next.is_null() { (*next).prev = prev; }
                self.free_at_order[order] -= 1;
                return true;
            }
            cur = (*cur).next;
        }
        false
    }

    /// Pop the head block from free_lists[order].
    unsafe fn list_pop(&mut self, order: usize) -> Option<u64> {
        let head = self.free_lists[order];
        if head.is_null() { return None; }
        let next = (*head).next;
        self.free_lists[order] = next;
        if !next.is_null() { (*next).prev = core::ptr::null_mut(); }
        self.free_at_order[order] -= 1;
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
        let cur   = addr;
        let mut cur_k = top;
        while cur_k > order {
            cur_k -= 1;
            let spare = cur + (1u64 << cur_k) * PAGE_SIZE;
            unsafe { self.list_push(spare, cur_k); }
        }

        self.allocated_pages += 1u64 << order;
        self.free_pages = self.free_pages.saturating_sub(1u64 << order);
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
                let base     = cur as u64;
                let base_end = base.saturating_add(k_bytes);
                if base.saturating_add(block_bytes) <= limit.saturating_add(1)
                    && base_end <= limit.saturating_add(1)
                {
                    // Carve `order` pages from this block.
                    unsafe { self.list_remove(base, k); }
                    // Temporarily credit these pages as free so split
                    // list_pushes don't over-subtract.
                    self.free_pages += 1u64 << k;

                    let current = base;
                    let mut current_k = k;
                    while current_k > order {
                        current_k -= 1;
                        let spare = current + (1u64 << current_k) * PAGE_SIZE;
                        unsafe { self.list_push(spare, current_k); }
                        self.free_pages += 1u64 << current_k; // list_push doesn't track
                    }

                    // Net: allocated = base block - all spares
                    let spares_pages = (1u64 << k) - (1u64 << order);
                    self.free_pages = self.free_pages.saturating_sub(1u64 << k);
                    self.free_pages += spares_pages;
                    self.allocated_pages += 1u64 << order;
                    return Ok(current);
                }
                unsafe { cur = (*cur).next; }
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
            let size_order  = usize::BITS as usize - 1 - remaining.leading_zeros() as usize;
            let order = align_order.min(size_order).min(MAX_ORDER);
            // SAFETY: identity-mapped physical range.
            unsafe { self.carve_block(base, order); }
            self.allocated_pages += 1u64 << order;
            self.free_pages = self.free_pages.saturating_sub(1u64 << order);
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
        for k in (order + 1)..=MAX_ORDER {
            let k_bytes   = (1u64 << k) * PAGE_SIZE;
            let container = addr & !(k_bytes - 1);
            if self.list_remove(container, k) {
                self.free_pages += 1u64 << k; // will be consumed by list_pushes below

                let mut current   = container;
                let mut current_k = k;
                while current_k > order {
                    current_k -= 1;
                    let half_bytes = (1u64 << current_k) * PAGE_SIZE;
                    let spare = current + half_bytes;
                    if addr >= spare {
                        self.list_push(current, current_k);
                        self.free_pages += 1u64 << current_k;
                        current = spare;
                    } else {
                        self.list_push(spare, current_k);
                        self.free_pages += 1u64 << current_k;
                    }
                }
                self.free_pages = self.free_pages.saturating_sub(1u64 << k);
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
        self.free_pages += 1u64 << current_k;
        // The coalesced free_pages from list_remove (buddy) calls were already
        // subtracted by list_remove not tracking free_pages — we only track
        // the final net addition here.
    }

    /// Decompose physical range [base, end) into natural buddy blocks and add
    /// each to the appropriate free list.
    unsafe fn buddy_add_range(&mut self, base: u64, end: u64) {
        let mut cur = base;
        while cur < end {
            let remaining_pages = (end - cur) / PAGE_SIZE;
            if remaining_pages == 0 { break; }
            let align_order = (cur.trailing_zeros() as usize).saturating_sub(PAGE_SHIFT as usize).min(MAX_ORDER);
            let size_order  = (usize::BITS as usize - 1 - remaining_pages.leading_zeros() as usize).min(MAX_ORDER);
            let order = align_order.min(size_order);
            self.list_push(cur, order);
            self.free_pages += 1u64 << order;
            cur += (1u64 << order) * PAGE_SIZE;
        }
    }

    /// Remove a single order-0 page at `addr` from the buddy free lists,
    /// carving it out of a larger block if needed.
    unsafe fn buddy_remove_page(&mut self, addr: u64) {
        self.carve_block(addr, 0);
        self.free_pages = self.free_pages.saturating_sub(1);
    }

    // ─────────────────────────────────────────────────────────────────────
    // SNAPSHOT MAP MAINTENANCE  (used for type queries / E820 only)
    // ─────────────────────────────────────────────────────────────────────

    fn map_snapshot_add(&mut self, addr: u64, pages: u64, mem_type: MemoryType) {
        // Update existing overlapping free descriptor if found.
        for i in 0..self.map_count {
            if self.map[i].mem_type.is_free() && self.map[i].contains(addr) {
                self.map[i].mem_type        = mem_type;
                self.map[i].physical_start  = addr;
                self.map[i].number_of_pages = pages;
                return;
            }
        }
        // Append new entry.
        if self.map_count < MAX_MAP {
            self.map[self.map_count] = MemoryDescriptor {
                mem_type,
                physical_start:  addr,
                virtual_start:   0,
                number_of_pages: pages,
                attribute:       MemoryAttribute::WB,
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

    // ─────────────────────────────────────────────────────────────────────
    // DIAGNOSTICS
    // ─────────────────────────────────────────────────────────────────────

    fn print_summary(&self) {
        let total_mb = (self.total_pages * PAGE_SIZE) >> 20;
        let free_mb  = (self.free_pages  * PAGE_SIZE) >> 20;
        puts("[MEM] buddy allocator ready: total=");
        put_hex32(total_mb as u32);
        puts("MB free=");
        put_hex32(free_mb as u32);
        puts("MB regions=");
        put_hex32(self.map_count as u32);
        puts("\n");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ORDER ARITHMETIC
// ═══════════════════════════════════════════════════════════════════════════

/// Smallest order k such that 2ᵏ ≥ pages.
#[inline]
fn pages_to_order(pages: u64) -> usize {
    if pages <= 1 { return 0; }
    let p2 = pages.next_power_of_two();
    (p2.trailing_zeros() as usize).min(MAX_ORDER)
}

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL REGISTRY
// ═══════════════════════════════════════════════════════════════════════════

static mut GLOBAL_REGISTRY:       MemoryRegistry = MemoryRegistry::new();
static mut REGISTRY_INITIALIZED:  bool           = false;

/// Parse and ingest the UEFI memory map into the global registry.
///
/// # Safety
/// Call exactly once, immediately after `ExitBootServices`, single-threaded.
pub unsafe fn init_global_registry(
    map_ptr:            *const u8,
    map_size:           usize,
    descriptor_size:    usize,
    descriptor_version: u32,
) {
    if REGISTRY_INITIALIZED {
        puts("[MEM] WARNING: registry already initialized!\n");
        return;
    }
    GLOBAL_REGISTRY.import_uefi_map(map_ptr, map_size, descriptor_size, descriptor_version);
    REGISTRY_INITIALIZED = true;
}

/// # Safety: bare-metal, single-threaded.
pub unsafe fn global_registry()     -> &'static     MemoryRegistry { &GLOBAL_REGISTRY }
/// # Safety: bare-metal, single-threaded.
pub unsafe fn global_registry_mut() -> &'static mut MemoryRegistry { &mut GLOBAL_REGISTRY }

pub fn is_registry_initialized() -> bool { unsafe { REGISTRY_INITIALIZED } }

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY COMPATIBILITY SHIMS
// ═══════════════════════════════════════════════════════════════════════════

/// Type alias — MemoryRegistry was previously called PhysicalMemoryMap in some callers.
pub type PhysicalMemoryMap = MemoryRegistry;

/// Type alias — MemoryDescriptor was previously called MemoryRegion.
pub type MemoryRegion = MemoryDescriptor;

/// Minimal bump allocator for early pre-registry use (rarely needed now).
pub struct PhysicalAllocator {
    current: u64,
    end:     u64,
}

impl PhysicalAllocator {
    pub const fn new(base: u64, size: u64) -> Self {
        Self { current: base, end: base.wrapping_add(size) }
    }

    pub fn alloc_pages(&mut self, count: usize) -> Option<u64> {
        let aligned = (self.current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let size    = (count as u64) * PAGE_SIZE;
        if aligned + size > self.end { return None; }
        self.current = aligned + size;
        Some(aligned)
    }

    pub fn alloc_bytes(&mut self, size: usize) -> Option<u64> {
        let aligned = (self.current + 15) & !15;
        let end     = aligned + size as u64;
        if end > self.end { return None; }
        self.current = end;
        Some(aligned)
    }

    pub fn remaining(&self) -> u64 { self.end.saturating_sub(self.current) }
}

/// Parse a UEFI memory map and return a standalone (non-global) registry.
///
/// # Safety: Same as `import_uefi_map`.
pub unsafe fn parse_uefi_memory_map(
    map_ptr:   *const u8,
    map_size:  usize,
    desc_size: usize,
) -> MemoryRegistry {
    let mut r = MemoryRegistry::new();
    r.import_uefi_map(map_ptr, map_size, desc_size, 1);
    r
}

pub fn fallback_allocator() -> PhysicalAllocator {
    puts("[MEM] WARNING: using fallback allocator (16MB-32MB)\n");
    PhysicalAllocator::new(0x0100_0000, 16 * 1024 * 1024)
}
