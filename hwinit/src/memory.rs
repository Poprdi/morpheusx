//! Memory Registry - Our Own Memory Services
//!
//! After ExitBootServices, UEFI's memory services are GONE. We become the
//! memory authority. This module mirrors UEFI's memory service API but with
//! our own implementation.
//!
//! # Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                      MEMORY REGISTRY                                │
//! │                                                                     │
//! │  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐               │
//! │  │  Regions[]  │   │ Allocator   │   │  E820 Gen   │               │
//! │  │  (the map)  │   │ (bump/free) │   │ (for Linux) │               │
//! │  └─────────────┘   └─────────────┘   └─────────────┘               │
//! │         │                 │                 │                       │
//! │         └────────────────┼─────────────────┘                       │
//! │                          │                                          │
//! │                    ┌─────▼─────┐                                    │
//! │                    │ Services  │                                    │
//! │                    │           │                                    │
//! │                    │ • alloc   │                                    │
//! │                    │ • free    │                                    │
//! │                    │ • query   │                                    │
//! │                    │ • export  │                                    │
//! │                    └───────────┘                                    │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # API (mirrors UEFI where sensible)
//!
//! | UEFI Service          | Our Equivalent                |
//! |-----------------------|-------------------------------|
//! | GetMemoryMap          | get_memory_map()              |
//! | AllocatePages         | allocate_pages()              |
//! | FreePages             | free_pages()                  |
//! | AllocatePool          | allocate_pool()               |
//! | FreePool              | free_pool()                   |
//! | SetMemoryAttributes   | (not needed post-EBS)         |

use crate::serial::{puts, put_hex64, put_hex32};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Page size (4KB, same as UEFI)
pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_SHIFT: u32 = 12;

/// Maximum memory regions we track
const MAX_REGIONS: usize = 256;

/// Maximum free list entries (for proper free/alloc)
const MAX_FREE_LIST: usize = 128;

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Memory type - mirrors UEFI's EFI_MEMORY_TYPE but we own it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MemoryType {
    /// Reserved by firmware. Do not touch.
    Reserved = 0,
    /// Loader code - reclaimable after we're done
    LoaderCode = 1,
    /// Loader data - reclaimable after we're done
    LoaderData = 2,
    /// Boot services code - reclaimable (we're past EBS)
    BootServicesCode = 3,
    /// Boot services data - reclaimable (we're past EBS)
    BootServicesData = 4,
    /// Runtime services code - preserve for UEFI runtime
    RuntimeServicesCode = 5,
    /// Runtime services data - preserve for UEFI runtime
    RuntimeServicesData = 6,
    /// Conventional memory - FREE, use it
    Conventional = 7,
    /// Unusable/bad memory
    Unusable = 8,
    /// ACPI reclaim memory - reclaimable after ACPI parse
    AcpiReclaim = 9,
    /// ACPI NVS - must preserve across sleep
    AcpiNvs = 10,
    /// Memory-mapped I/O
    Mmio = 11,
    /// Memory-mapped I/O port space
    MmioPortSpace = 12,
    /// Processor reserved (PAL code on Itanium)
    PalCode = 13,
    /// Persistent memory (NVDIMM)
    Persistent = 14,

    // Our custom types (above UEFI range)
    /// Allocated by us for DMA
    AllocatedDma = 0x8000_0001,
    /// Allocated by us for stacks
    AllocatedStack = 0x8000_0002,
    /// Allocated by us for page tables
    AllocatedPageTable = 0x8000_0003,
    /// Allocated by us for heap
    AllocatedHeap = 0x8000_0004,
    /// Allocated by us (generic)
    Allocated = 0x8000_0000,
}

impl MemoryType {
    /// Convert from raw UEFI memory type value.
    pub fn from_uefi_raw(value: u32) -> Self {
        match value {
            0 => MemoryType::Reserved,
            1 => MemoryType::LoaderCode,
            2 => MemoryType::LoaderData,
            3 => MemoryType::BootServicesCode,
            4 => MemoryType::BootServicesData,
            5 => MemoryType::RuntimeServicesCode,
            6 => MemoryType::RuntimeServicesData,
            7 => MemoryType::Conventional,
            8 => MemoryType::Unusable,
            9 => MemoryType::AcpiReclaim,
            10 => MemoryType::AcpiNvs,
            11 => MemoryType::Mmio,
            12 => MemoryType::MmioPortSpace,
            13 => MemoryType::PalCode,
            14 => MemoryType::Persistent,
            _ => MemoryType::Reserved,
        }
    }

    /// Is this memory type free for general use?
    pub fn is_free(&self) -> bool {
        matches!(self,
            MemoryType::Conventional |
            MemoryType::LoaderCode |
            MemoryType::LoaderData |
            MemoryType::BootServicesCode |
            MemoryType::BootServicesData
        )
    }

    /// Is this memory type usable after ACPI init?
    pub fn is_reclaimable(&self) -> bool {
        matches!(self, MemoryType::AcpiReclaim)
    }

    /// Must this memory be preserved?
    pub fn must_preserve(&self) -> bool {
        matches!(self,
            MemoryType::Reserved |
            MemoryType::RuntimeServicesCode |
            MemoryType::RuntimeServicesData |
            MemoryType::AcpiNvs |
            MemoryType::Mmio |
            MemoryType::MmioPortSpace |
            MemoryType::PalCode |
            MemoryType::Unusable
        )
    }

    /// Convert to E820 type for Linux handoff.
    pub fn to_e820(&self) -> E820Type {
        match self {
            MemoryType::Conventional |
            MemoryType::LoaderCode |
            MemoryType::LoaderData |
            MemoryType::BootServicesCode |
            MemoryType::BootServicesData |
            MemoryType::Allocated |
            MemoryType::AllocatedDma |
            MemoryType::AllocatedStack |
            MemoryType::AllocatedPageTable |
            MemoryType::AllocatedHeap => E820Type::Ram,

            MemoryType::AcpiReclaim => E820Type::Acpi,
            MemoryType::AcpiNvs => E820Type::Nvs,
            MemoryType::Persistent => E820Type::Pmem,
            MemoryType::Unusable => E820Type::Unusable,

            _ => E820Type::Reserved,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY ATTRIBUTES (mirrors UEFI)
// ═══════════════════════════════════════════════════════════════════════════

/// Memory attributes - bitflags, same as UEFI EFI_MEMORY_ATTRIBUTE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAttribute(pub u64);

impl MemoryAttribute {
    // Cache attributes (mutually exclusive)
    pub const UC: Self = Self(0x0000_0000_0000_0001);  // Uncacheable
    pub const WC: Self = Self(0x0000_0000_0000_0002);  // Write-combining
    pub const WT: Self = Self(0x0000_0000_0000_0004);  // Write-through
    pub const WB: Self = Self(0x0000_0000_0000_0008);  // Write-back
    pub const UCE: Self = Self(0x0000_0000_0000_0010); // Uncacheable, exported

    // Physical memory protection
    pub const WP: Self = Self(0x0000_0000_0000_1000);  // Write-protected
    pub const RP: Self = Self(0x0000_0000_0000_2000);  // Read-protected
    pub const XP: Self = Self(0x0000_0000_0000_4000);  // Execute-protected
    pub const NV: Self = Self(0x0000_0000_0000_8000);  // Non-volatile

    // UEFI 2.5+
    pub const MORE_RELIABLE: Self = Self(0x0000_0000_0001_0000);
    pub const RO: Self = Self(0x0000_0000_0002_0000);  // Read-only
    pub const SP: Self = Self(0x0000_0000_0004_0000);  // Specific-purpose

    // Runtime
    pub const RUNTIME: Self = Self(0x8000_0000_0000_0000); // Needs runtime mapping

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// E820 TYPES (for Linux)
// ═══════════════════════════════════════════════════════════════════════════

/// E820 memory type for Linux boot protocol.
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

/// E820 entry - matches Linux boot protocol format.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub entry_type: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY DESCRIPTOR (mirrors UEFI EFI_MEMORY_DESCRIPTOR)
// ═══════════════════════════════════════════════════════════════════════════

/// Memory region descriptor - our version of EFI_MEMORY_DESCRIPTOR.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryDescriptor {
    /// Memory type
    pub mem_type: MemoryType,
    /// Physical start address (page-aligned)
    pub physical_start: u64,
    /// Virtual start address (for runtime services)
    pub virtual_start: u64,
    /// Number of 4KB pages
    pub number_of_pages: u64,
    /// Memory attributes
    pub attribute: MemoryAttribute,
}

impl MemoryDescriptor {
    /// Create empty descriptor.
    pub const fn empty() -> Self {
        Self {
            mem_type: MemoryType::Reserved,
            physical_start: 0,
            virtual_start: 0,
            number_of_pages: 0,
            attribute: MemoryAttribute::empty(),
        }
    }

    /// Physical end address (exclusive).
    pub const fn physical_end(&self) -> u64 {
        self.physical_start + (self.number_of_pages * PAGE_SIZE)
    }

    /// Size in bytes.
    pub const fn size(&self) -> u64 {
        self.number_of_pages * PAGE_SIZE
    }

    /// Check if address is within this region.
    pub const fn contains(&self, addr: u64) -> bool {
        addr >= self.physical_start && addr < self.physical_end()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// FREE LIST ENTRY (for allocation tracking)
// ═══════════════════════════════════════════════════════════════════════════

/// Free memory block for allocation.
#[derive(Debug, Clone, Copy)]
struct FreeBlock {
    base: u64,
    pages: u64,
}

impl FreeBlock {
    const fn empty() -> Self {
        Self { base: 0, pages: 0 }
    }

    const fn is_empty(&self) -> bool {
        self.pages == 0
    }

    const fn size(&self) -> u64 {
        self.pages * PAGE_SIZE
    }

    const fn end(&self) -> u64 {
        self.base + self.size()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ALLOCATION TYPE (mirrors UEFI EFI_ALLOCATE_TYPE)
// ═══════════════════════════════════════════════════════════════════════════

/// Allocation type - how to choose the allocation address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateType {
    /// Allocate any available pages
    AnyPages,
    /// Allocate at highest available address <= specified
    MaxAddress(u64),
    /// Allocate at exactly the specified address
    Address(u64),
}

// ═══════════════════════════════════════════════════════════════════════════
// ERROR TYPE
// ═══════════════════════════════════════════════════════════════════════════

/// Memory service errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    /// Out of resources
    OutOfResources,
    /// Invalid parameter
    InvalidParameter,
    /// Not found
    NotFound,
    /// Buffer too small
    BufferTooSmall,
    /// Already allocated
    AlreadyAllocated,
}

// ═══════════════════════════════════════════════════════════════════════════
// MEMORY REGISTRY
// ═══════════════════════════════════════════════════════════════════════════

/// The Memory Registry - our replacement for UEFI memory services.
///
/// This is THE authority for memory after ExitBootServices.
pub struct MemoryRegistry {
    /// The canonical memory map (all regions)
    map: [MemoryDescriptor; MAX_REGIONS],
    map_count: usize,
    /// Map key (increments on changes, like UEFI)
    map_key: u64,

    /// Free list for allocations
    free_list: [FreeBlock; MAX_FREE_LIST],
    free_count: usize,

    /// Bump allocator fallback (for simple/fast allocs)
    bump_base: u64,
    bump_current: u64,
    bump_limit: u64,

    /// Statistics
    total_pages: u64,
    free_pages: u64,
    allocated_pages: u64,
}

impl MemoryRegistry {
    /// Create empty registry.
    pub const fn new() -> Self {
        Self {
            map: [MemoryDescriptor::empty(); MAX_REGIONS],
            map_count: 0,
            map_key: 0,

            free_list: [FreeBlock::empty(); MAX_FREE_LIST],
            free_count: 0,

            bump_base: 0,
            bump_current: 0,
            bump_limit: 0,

            total_pages: 0,
            free_pages: 0,
            allocated_pages: 0,
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // INITIALIZATION (import from UEFI)
    // ─────────────────────────────────────────────────────────────────────

    /// Import UEFI memory map and take ownership.
    ///
    /// After this call, we are the memory authority.
    ///
    /// # Safety
    /// - map_ptr must point to valid UEFI memory map
    /// - Called exactly once, immediately after ExitBootServices
    pub unsafe fn import_uefi_map(
        &mut self,
        map_ptr: *const u8,
        map_size: usize,
        descriptor_size: usize,
        _descriptor_version: u32,
    ) {
        let entry_count = map_size / descriptor_size;

        puts("[MEM] importing UEFI map: ");
        put_hex32(entry_count as u32);
        puts(" entries, desc_size=");
        put_hex32(descriptor_size as u32);
        puts("\n");

        // Parse UEFI descriptors into our format
        for i in 0..entry_count {
            if self.map_count >= MAX_REGIONS {
                puts("[MEM] WARNING: region limit reached\n");
                break;
            }

            let entry_ptr = map_ptr.add(i * descriptor_size);

            // UEFI EFI_MEMORY_DESCRIPTOR layout (v1):
            // offset 0:  u32 Type
            // offset 4:  u32 Padding (on 64-bit)
            // offset 8:  u64 PhysicalStart
            // offset 16: u64 VirtualStart
            // offset 24: u64 NumberOfPages
            // offset 32: u64 Attribute

            let uefi_type = *(entry_ptr as *const u32);
            let phys_start = *(entry_ptr.add(8) as *const u64);
            let virt_start = *(entry_ptr.add(16) as *const u64);
            let num_pages = *(entry_ptr.add(24) as *const u64);
            let attribute = *(entry_ptr.add(32) as *const u64);

            let mem_type = MemoryType::from_uefi_raw(uefi_type);

            self.map[self.map_count] = MemoryDescriptor {
                mem_type,
                physical_start: phys_start,
                virtual_start: virt_start,
                number_of_pages: num_pages,
                attribute: MemoryAttribute(attribute),
            };

            self.total_pages += num_pages;
            if mem_type.is_free() {
                self.free_pages += num_pages;
            }

            self.map_count += 1;
        }

        self.map_key = 1;

        // Build free list from conventional memory
        self.rebuild_free_list();

        // Set up bump allocator from largest free block below 4GB
        self.init_bump_allocator();

        self.print_summary();
    }

    /// Rebuild free list from current map.
    fn rebuild_free_list(&mut self) {
        self.free_count = 0;

        for i in 0..self.map_count {
            let desc = &self.map[i];
            if !desc.mem_type.is_free() {
                continue;
            }

            if self.free_count >= MAX_FREE_LIST {
                break;
            }

            self.free_list[self.free_count] = FreeBlock {
                base: desc.physical_start,
                pages: desc.number_of_pages,
            };
            self.free_count += 1;
        }

        // Sort by base address (simple bubble sort, list is small)
        for i in 0..self.free_count {
            for j in 0..(self.free_count - 1 - i) {
                if self.free_list[j].base > self.free_list[j + 1].base {
                    let tmp = self.free_list[j];
                    self.free_list[j] = self.free_list[j + 1];
                    self.free_list[j + 1] = tmp;
                }
            }
        }
    }

    /// Initialize bump allocator from largest free region below 4GB.
    fn init_bump_allocator(&mut self) {
        let mut best_idx = None;
        let mut best_size = 0u64;

        for i in 0..self.free_count {
            let block = &self.free_list[i];

            // Must be below 4GB for DMA
            if block.base >= 0x1_0000_0000 {
                continue;
            }

            // Clamp to 4GB boundary
            let usable_end = block.end().min(0x1_0000_0000);
            let usable_size = usable_end.saturating_sub(block.base);

            // Want at least 1MB
            if usable_size < 0x10_0000 {
                continue;
            }

            if usable_size > best_size {
                best_idx = Some(i);
                best_size = usable_size;
            }
        }

        if let Some(idx) = best_idx {
            let block = &self.free_list[idx];
            let mut base = block.base;

            // Skip first 1MB (legacy BIOS area)
            if base < 0x10_0000 {
                let skip = 0x10_0000 - base;
                base = 0x10_0000;
                best_size = best_size.saturating_sub(skip);
            }

            self.bump_base = base;
            self.bump_current = base;
            self.bump_limit = base + best_size;

            puts("[MEM] bump allocator: ");
            put_hex64(base);
            puts(" - ");
            put_hex64(base + best_size);
            puts(" (");
            put_hex32((best_size / (1024 * 1024)) as u32);
            puts(" MB)\n");
        }
    }

    fn print_summary(&self) {
        let total_mb = (self.total_pages * PAGE_SIZE) / (1024 * 1024);
        let free_mb = (self.free_pages * PAGE_SIZE) / (1024 * 1024);

        puts("[MEM] total: ");
        put_hex32(total_mb as u32);
        puts(" MB, free: ");
        put_hex32(free_mb as u32);
        puts(" MB, regions: ");
        put_hex32(self.map_count as u32);
        puts("\n");
    }

    // ─────────────────────────────────────────────────────────────────────
    // UEFI-STYLE SERVICES
    // ─────────────────────────────────────────────────────────────────────

    /// Get the current memory map.
    ///
    /// Mirrors UEFI GetMemoryMap().
    ///
    /// # Returns
    /// (map_key, descriptor_count)
    pub fn get_memory_map(&self) -> (u64, usize) {
        (self.map_key, self.map_count)
    }

    /// Get memory map key (changes on allocation/free).
    pub fn get_map_key(&self) -> u64 {
        self.map_key
    }

    /// Get descriptor by index.
    pub fn get_descriptor(&self, index: usize) -> Option<&MemoryDescriptor> {
        if index < self.map_count {
            Some(&self.map[index])
        } else {
            None
        }
    }

    /// Allocate pages.
    ///
    /// Mirrors UEFI AllocatePages().
    pub fn allocate_pages(
        &mut self,
        alloc_type: AllocateType,
        mem_type: MemoryType,
        pages: u64,
    ) -> Result<u64, MemoryError> {
        if pages == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        let addr = match alloc_type {
            AllocateType::AnyPages => self.alloc_any_pages(pages)?,
            AllocateType::MaxAddress(max) => self.alloc_max_address(pages, max)?,
            AllocateType::Address(addr) => {
                self.alloc_at_address(addr, pages)?;
                addr
            }
        };

        // Update map to reflect allocation
        self.mark_allocated(addr, pages, mem_type);

        Ok(addr)
    }

    /// Free previously allocated pages.
    ///
    /// Mirrors UEFI FreePages().
    pub fn free_pages(&mut self, addr: u64, pages: u64) -> Result<(), MemoryError> {
        if addr & (PAGE_SIZE - 1) != 0 {
            return Err(MemoryError::InvalidParameter);
        }

        // Find and update the region
        for i in 0..self.map_count {
            let desc = &mut self.map[i];
            if desc.physical_start == addr && desc.number_of_pages == pages {
                desc.mem_type = MemoryType::Conventional;
                self.free_pages += pages;
                self.allocated_pages -= pages;
                self.map_key += 1;

                // TODO: coalesce with adjacent free regions
                self.rebuild_free_list();
                return Ok(());
            }
        }

        Err(MemoryError::NotFound)
    }

    /// Allocate pool memory (byte-granular).
    ///
    /// Mirrors UEFI AllocatePool() - uses bump allocator for speed.
    pub fn allocate_pool(&mut self, size: usize) -> Result<u64, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidParameter);
        }

        // 16-byte alignment for DMA compatibility
        let aligned = (self.bump_current + 15) & !15;
        let needed = aligned + size as u64;

        if needed > self.bump_limit {
            return Err(MemoryError::OutOfResources);
        }

        self.bump_current = needed;
        Ok(aligned)
    }

    // ─────────────────────────────────────────────────────────────────────
    // INTERNAL ALLOCATION HELPERS
    // ─────────────────────────────────────────────────────────────────────

    fn alloc_any_pages(&mut self, pages: u64) -> Result<u64, MemoryError> {
        // Prefer bump allocator if enough space
        let needed = pages * PAGE_SIZE;
        let aligned = (self.bump_current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        if aligned + needed <= self.bump_limit {
            self.bump_current = aligned + needed;
            return Ok(aligned);
        }

        // Fall back to free list
        for i in 0..self.free_count {
            let block = &mut self.free_list[i];
            if block.pages >= pages {
                let addr = block.base;

                // Shrink block
                block.base += pages * PAGE_SIZE;
                block.pages -= pages;

                return Ok(addr);
            }
        }

        Err(MemoryError::OutOfResources)
    }

    fn alloc_max_address(&mut self, pages: u64, max_addr: u64) -> Result<u64, MemoryError> {
        // Find highest suitable block
        let mut best_idx = None;
        let mut best_addr = 0u64;

        for i in 0..self.free_count {
            let block = &self.free_list[i];

            if block.pages < pages {
                continue;
            }

            // Check if we can fit below max_addr
            let alloc_end = block.base + pages * PAGE_SIZE;
            if alloc_end <= max_addr && block.base > best_addr {
                best_idx = Some(i);
                best_addr = block.base;
            }
        }

        if let Some(i) = best_idx {
            let block = &mut self.free_list[i];
            let addr = block.base;
            block.base += pages * PAGE_SIZE;
            block.pages -= pages;
            Ok(addr)
        } else {
            Err(MemoryError::OutOfResources)
        }
    }

    fn alloc_at_address(&mut self, addr: u64, pages: u64) -> Result<(), MemoryError> {
        if addr & (PAGE_SIZE - 1) != 0 {
            return Err(MemoryError::InvalidParameter);
        }

        let end = addr + pages * PAGE_SIZE;

        // Find containing free block
        for i in 0..self.free_count {
            let block = &mut self.free_list[i];

            if addr >= block.base && end <= block.end() {
                // Split block if necessary
                // For simplicity, we just shrink it
                // TODO: proper splitting

                if addr == block.base {
                    block.base = end;
                    block.pages -= pages;
                } else {
                    // Just mark as smaller (lose some space)
                    block.pages = (addr - block.base) / PAGE_SIZE;
                }

                return Ok(());
            }
        }

        Err(MemoryError::NotFound)
    }

    fn mark_allocated(&mut self, addr: u64, pages: u64, mem_type: MemoryType) {
        // Add new region for this allocation
        if self.map_count < MAX_REGIONS {
            self.map[self.map_count] = MemoryDescriptor {
                mem_type,
                physical_start: addr,
                virtual_start: 0,
                number_of_pages: pages,
                attribute: MemoryAttribute::WB, // Default to write-back
            };
            self.map_count += 1;
        }

        self.free_pages -= pages;
        self.allocated_pages += pages;
        self.map_key += 1;
    }

    // ─────────────────────────────────────────────────────────────────────
    // QUERY FUNCTIONS
    // ─────────────────────────────────────────────────────────────────────

    /// Total physical memory.
    pub fn total_memory(&self) -> u64 {
        self.total_pages * PAGE_SIZE
    }

    /// Total free memory.
    pub fn free_memory(&self) -> u64 {
        self.free_pages * PAGE_SIZE
    }

    /// Total allocated memory.
    pub fn allocated_memory(&self) -> u64 {
        self.allocated_pages * PAGE_SIZE
    }

    /// Remaining bump allocator space.
    pub fn bump_remaining(&self) -> u64 {
        self.bump_limit.saturating_sub(self.bump_current)
    }

    /// Find the largest free region below 4GB.
    ///
    /// Returns (base_address, size_in_bytes) or None if nothing found.
    /// Used for DMA region allocation where we need addressable memory.
    pub fn find_largest_free_below_4gb(&self) -> Option<(u64, u64)> {
        let mut best_base = 0u64;
        let mut best_size = 0u64;

        for i in 0..self.free_count {
            let block = &self.free_list[i];

            // Must start below 4GB
            if block.base >= 0x1_0000_0000 {
                continue;
            }

            // Clamp end to 4GB
            let usable_end = block.end().min(0x1_0000_0000);

            // Skip first 1MB (legacy area)
            let start = if block.base < 0x10_0000 { 0x10_0000 } else { block.base };
            let adjusted_size = usable_end.saturating_sub(start);

            if adjusted_size > best_size {
                best_base = start;
                best_size = adjusted_size;
            }
        }

        if best_size > 0 {
            Some((best_base, best_size))
        } else {
            None
        }
    }

    /// Find memory type at address.
    pub fn memory_type_at(&self, addr: u64) -> MemoryType {
        for i in 0..self.map_count {
            if self.map[i].contains(addr) {
                return self.map[i].mem_type;
            }
        }
        MemoryType::Reserved // Unknown = reserved
    }

    // ─────────────────────────────────────────────────────────────────────
    // E820 EXPORT (for Linux)
    // ─────────────────────────────────────────────────────────────────────

    /// Export memory map as E820 for Linux boot protocol.
    ///
    /// # Returns
    /// Number of entries written.
    pub fn export_e820(&self, buffer: &mut [E820Entry]) -> usize {
        let mut count = 0;

        for i in 0..self.map_count {
            if count >= buffer.len() {
                break;
            }

            let desc = &self.map[i];
            buffer[count] = E820Entry {
                addr: desc.physical_start,
                size: desc.size(),
                entry_type: desc.mem_type.to_e820() as u32,
            };
            count += 1;
        }

        // TODO: coalesce adjacent same-type regions for cleaner E820

        count
    }

    /// Get E820 entry count.
    pub fn e820_count(&self) -> usize {
        self.map_count
    }

    // ─────────────────────────────────────────────────────────────────────
    // CONVENIENCE ALLOCATORS
    // ─────────────────────────────────────────────────────────────────────

    /// Allocate DMA-suitable pages (below 4GB, page-aligned).
    pub fn alloc_dma_pages(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(
            AllocateType::MaxAddress(0xFFFF_FFFF),
            MemoryType::AllocatedDma,
            pages,
        )
    }

    /// Allocate DMA-suitable bytes (below 4GB, 16-byte aligned).
    pub fn alloc_dma_bytes(&mut self, size: usize) -> Result<u64, MemoryError> {
        // For small allocations, use bump allocator
        if size < PAGE_SIZE as usize && self.bump_limit < 0x1_0000_0000 {
            return self.allocate_pool(size);
        }

        // For larger, allocate full pages
        let pages = (size as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        self.alloc_dma_pages(pages)
    }

    /// Allocate stack pages.
    pub fn alloc_stack(&mut self, pages: u64) -> Result<u64, MemoryError> {
        self.allocate_pages(
            AllocateType::AnyPages,
            MemoryType::AllocatedStack,
            pages,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL REGISTRY (optional static access)
// ═══════════════════════════════════════════════════════════════════════════

/// Global memory registry.
///
/// In a bare-metal environment without threads, this is safe.
static mut GLOBAL_REGISTRY: MemoryRegistry = MemoryRegistry::new();
static mut REGISTRY_INITIALIZED: bool = false;

/// Initialize the global memory registry from UEFI map.
///
/// # Safety
/// - Call exactly once, immediately after ExitBootServices
/// - Single-threaded context only
pub unsafe fn init_global_registry(
    map_ptr: *const u8,
    map_size: usize,
    descriptor_size: usize,
    descriptor_version: u32,
) {
    if REGISTRY_INITIALIZED {
        puts("[MEM] WARNING: registry already initialized!\n");
        return;
    }

    GLOBAL_REGISTRY.import_uefi_map(map_ptr, map_size, descriptor_size, descriptor_version);
    REGISTRY_INITIALIZED = true;
}

/// Get reference to global registry.
///
/// # Safety
/// Must be initialized first.
pub unsafe fn global_registry() -> &'static MemoryRegistry {
    &GLOBAL_REGISTRY
}

/// Get mutable reference to global registry.
///
/// # Safety
/// Must be initialized first. Single-threaded context only.
pub unsafe fn global_registry_mut() -> &'static mut MemoryRegistry {
    &mut GLOBAL_REGISTRY
}

/// Check if global registry is initialized.
pub fn is_registry_initialized() -> bool {
    unsafe { REGISTRY_INITIALIZED }
}

// ═══════════════════════════════════════════════════════════════════════════
// LEGACY COMPATIBILITY SHIMS
// ═══════════════════════════════════════════════════════════════════════════

// Keep old types for code that hasn't migrated yet

/// Legacy: alias for MemoryRegistry.
pub type PhysicalMemoryMap = MemoryRegistry;

/// Legacy: Simple allocator wrapper.
pub struct PhysicalAllocator {
    current: u64,
    end: u64,
}

impl PhysicalAllocator {
    pub const fn new(base: u64, size: u64) -> Self {
        Self {
            current: base,
            end: base.saturating_add(size),
        }
    }

    pub fn alloc_pages(&mut self, count: usize) -> Option<u64> {
        let size = (count as u64) * PAGE_SIZE;
        let aligned = (self.current + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        if aligned + size > self.end {
            return None;
        }

        self.current = aligned + size;
        Some(aligned)
    }

    pub fn alloc_bytes(&mut self, size: usize) -> Option<u64> {
        let aligned = (self.current + 15) & !15;

        if aligned + size as u64 > self.end {
            return None;
        }

        self.current = aligned + size as u64;
        Some(aligned)
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.current)
    }
}

/// Legacy: Parse UEFI map into registry.
pub unsafe fn parse_uefi_memory_map(
    map_ptr: *const u8,
    map_size: usize,
    desc_size: usize,
) -> MemoryRegistry {
    let mut registry = MemoryRegistry::new();
    registry.import_uefi_map(map_ptr, map_size, desc_size, 1);
    registry
}

/// Legacy: Fallback allocator.
pub fn fallback_allocator() -> PhysicalAllocator {
    puts("[MEM] WARNING: fallback allocator (16MB-32MB)\n");
    PhysicalAllocator::new(0x0100_0000, 16 * 1024 * 1024)
}

/// Legacy type alias
pub type MemoryRegion = MemoryDescriptor;
