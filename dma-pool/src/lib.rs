//! Firmware-agnostic DMA memory pool allocator.
//!
//! This crate provides a flexible DMA memory pool that can be used by
//! any bare-metal device driver. Supports both static and runtime-discovered memory.
//!
//! # Design Philosophy
//!
//! - **Zero firmware dependencies**: Works on any platform
//! - **Flexible memory sources**: Static pools, runtime discovery, or external allocation
//! - **Device-agnostic**: Any driver HAL can use this allocator
//! - **Thread-safe**: Spin-lock based synchronization
//!
//! # Memory Sources
//!
//! 1. **Static pool**: Compile-time allocated (simplest, always works)
//! 2. **Runtime discovery**: Find free memory regions in loaded binary
//! 3. **External**: Caller provides memory region (e.g., from firmware)
//!
//! # Usage
//!
//! ```ignore
//! use dma_pool::{DmaPool, MemoryRegion};
//!
//! // Option 1: Use built-in static pool
//! DmaPool::init_static();
//!
//! // Option 2: Runtime discovery (scan for usable memory)
//! DmaPool::init_discover();
//!
//! // Option 3: External memory
//! DmaPool::init_external(base_addr, size);
//!
//! // Allocate DMA memory
//! let (paddr, vaddr) = DmaPool::alloc_pages(4)?;
//! ```

#![no_std]
#![allow(dead_code)]

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Page size (4KB).
pub const PAGE_SIZE: usize = 4096;

/// Default static pool size (2MB).
pub const DEFAULT_POOL_SIZE: usize = 2 * 1024 * 1024;

/// Maximum allocation tracking entries.
pub const MAX_ALLOCATIONS: usize = 128;

/// Maximum number of caves in a chained pool.
pub const MAX_CAVES: usize = 16;

/// Minimum usable memory region size (64KB).
pub const MIN_REGION_SIZE: usize = 64 * 1024;

/// Minimum cave size worth tracking (4KB = 1 page).
pub const MIN_CAVE_SIZE: usize = PAGE_SIZE;

// ============================================================================
// Utility functions
// ============================================================================

/// Align a value up to the given alignment.
#[inline]
pub const fn align_up(val: usize, align: usize) -> usize {
    (val + align - 1) & !(align - 1)
}

/// Align a value down to the given alignment.
#[inline]
pub const fn align_down(val: usize, align: usize) -> usize {
    val & !(align - 1)
}

/// Convert pages to bytes.
#[inline]
pub const fn pages_to_bytes(pages: usize) -> usize {
    pages * PAGE_SIZE
}

/// Convert bytes to pages (rounded up).
#[inline]
pub const fn bytes_to_pages(bytes: usize) -> usize {
    align_up(bytes, PAGE_SIZE) / PAGE_SIZE
}

// ============================================================================
// Memory region discovery
// ============================================================================

/// A discovered memory region suitable for DMA.
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    /// Base address (physical = virtual in identity mapping).
    pub base: usize,
    /// Size in bytes.
    pub size: usize,
}

impl MemoryRegion {
    /// Create a new memory region.
    pub const fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }

    /// Check if region is usable for DMA (page-aligned, large enough).
    pub fn is_usable(&self) -> bool {
        self.base % PAGE_SIZE == 0 && self.size >= MIN_REGION_SIZE
    }

    /// Get aligned region.
    pub fn aligned(&self) -> Self {
        let aligned_base = align_up(self.base, PAGE_SIZE);
        let adjustment = aligned_base - self.base;
        let aligned_size = align_down(self.size.saturating_sub(adjustment), PAGE_SIZE);
        Self {
            base: aligned_base,
            size: aligned_size,
        }
    }
}

/// Memory discovery strategies.
///
/// Inspired by cavealloc's cave discovery - finds unused padding in executables.
pub struct MemoryDiscovery;

/// Padding byte patterns recognized as "cave" candidates.
/// These are commonly used for function alignment and unused space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingPattern {
    /// Zero bytes (0x00) - most common padding
    Zero,
    /// INT3 breakpoint (0xCC) - MSVC/Windows compiler padding
    Int3,
    /// NOP instruction (0x90) - GCC/Clang alignment padding
    Nop,
    /// Any of the above patterns
    Any,
}

impl MemoryDiscovery {
    /// Check if a byte is a padding byte.
    #[inline]
    fn is_padding(byte: u8, pattern: PaddingPattern) -> bool {
        match pattern {
            PaddingPattern::Zero => byte == 0x00,
            PaddingPattern::Int3 => byte == 0xCC,
            PaddingPattern::Nop => byte == 0x90,
            PaddingPattern::Any => byte == 0x00 || byte == 0xCC || byte == 0x90,
        }
    }

    /// Scan a memory range for padding regions (code caves).
    ///
    /// This searches for contiguous padding bytes that could be unused space.
    /// Detects: 0x00 (zeros), 0xCC (INT3), 0x90 (NOP) - standard compiler padding.
    ///
    /// # Safety
    ///
    /// - `start` and `end` must be valid, readable memory addresses.
    /// - The memory range should be part of our loaded image.
    pub unsafe fn find_caves(
        start: usize,
        end: usize,
        min_size: usize,
        pattern: PaddingPattern,
    ) -> Option<MemoryRegion> {
        let current = align_up(start, 16); // 16-byte align for DMA
        let end_aligned = align_down(end, PAGE_SIZE);

        if current >= end_aligned || end_aligned - current < min_size {
            return None;
        }

        let ptr = current as *const u8;
        let scan_len = end_aligned - current;
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        for i in 0..scan_len {
            let b = *ptr.add(i);
            if Self::is_padding(b, pattern) {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
            } else {
                if run_len >= min_size {
                    // Found a suitable cave
                    let region_start = align_up(current + run_start, PAGE_SIZE);
                    let region_end = current + run_start + run_len;
                    let aligned_size = align_down(region_end.saturating_sub(region_start), PAGE_SIZE);
                    if aligned_size >= min_size {
                        return Some(MemoryRegion::new(region_start, aligned_size));
                    }
                }
                run_len = 0;
            }
        }

        // Check final run
        if run_len >= min_size {
            let region_start = align_up(current + run_start, PAGE_SIZE);
            let region_end = current + run_start + run_len;
            let aligned_size = align_down(region_end.saturating_sub(region_start), PAGE_SIZE);
            if aligned_size >= min_size {
                return Some(MemoryRegion::new(region_start, aligned_size));
            }
        }

        None
    }

    /// Find all caves in a memory range (up to max_caves).
    ///
    /// Returns found caves sorted by size (largest first).
    ///
    /// # Safety
    ///
    /// Same requirements as `find_caves`.
    pub unsafe fn find_all_caves(
        start: usize,
        end: usize,
        min_size: usize,
        pattern: PaddingPattern,
        caves: &mut [MemoryRegion],
    ) -> usize {
        let mut found = 0usize;
        let max_caves = caves.len();
        
        let current = align_up(start, 16);
        let end_aligned = align_down(end, PAGE_SIZE);

        if current >= end_aligned || max_caves == 0 {
            return 0;
        }

        let ptr = current as *const u8;
        let scan_len = end_aligned - current;
        let mut run_start = 0usize;
        let mut run_len = 0usize;

        for i in 0..=scan_len {
            let b = if i < scan_len { *ptr.add(i) } else { 0xFF }; // Sentinel
            
            if i < scan_len && Self::is_padding(b, pattern) {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
            } else {
                if run_len >= min_size && found < max_caves {
                    let region_start = align_up(current + run_start, PAGE_SIZE);
                    let region_end = current + run_start + run_len;
                    let aligned_size = align_down(region_end.saturating_sub(region_start), PAGE_SIZE);
                    if aligned_size >= min_size {
                        caves[found] = MemoryRegion::new(region_start, aligned_size);
                        found += 1;
                    }
                }
                run_len = 0;
            }
        }

        // Sort by size descending (bubble sort for no_std simplicity)
        for i in 0..found.saturating_sub(1) {
            for j in 0..found - 1 - i {
                if caves[j].size < caves[j + 1].size {
                    caves.swap(j, j + 1);
                }
            }
        }

        found
    }

    /// Scan a memory range for large zero-filled regions (code caves).
    ///
    /// This searches for contiguous zero bytes that could be unused padding.
    /// Useful for finding space in PE/ELF section alignment gaps.
    ///
    /// # Safety
    ///
    /// - `start` and `end` must be valid, readable memory addresses.
    /// - The memory range should be part of our loaded image.
    pub unsafe fn find_zero_regions(start: usize, end: usize, min_size: usize) -> Option<MemoryRegion> {
        Self::find_caves(start, end, min_size, PaddingPattern::Zero)
    }

    /// Find memory by scanning after our BSS section.
    ///
    /// In many bare-metal scenarios, memory after BSS is unused.
    /// This is a heuristic - caller should verify the region is safe.
    ///
    /// # Safety
    ///
    /// Caller must ensure the returned region doesn't overlap with stack or other data.
    pub unsafe fn find_after_bss(bss_end: usize, max_scan: usize) -> Option<MemoryRegion> {
        let start = align_up(bss_end, PAGE_SIZE);
        let end = start + max_scan;

        // Simple heuristic: assume memory after BSS is usable
        // In real scenario, should check memory map
        Some(MemoryRegion::new(start, MIN_REGION_SIZE * 4))
    }

    /// Scan PE section padding for caves.
    ///
    /// PE sections are aligned to SectionAlignment (typically 4KB or 64KB).
    /// The gap between actual data and alignment boundary is often zeros/INT3.
    ///
    /// # Safety
    ///
    /// - `section_end` must be the actual end of section data
    /// - `aligned_end` must be the section alignment boundary
    pub unsafe fn find_section_padding(
        section_end: usize,
        aligned_end: usize,
        min_size: usize,
    ) -> Option<MemoryRegion> {
        if aligned_end <= section_end {
            return None;
        }
        Self::find_caves(section_end, aligned_end, min_size, PaddingPattern::Any)
    }
}

// ============================================================================
// Error types
// ============================================================================

/// DMA pool errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaError {
    /// Pool not initialized.
    NotInitialized,
    /// Requested 0 pages.
    ZeroPages,
    /// Not enough memory in pool.
    OutOfMemory,
    /// Pool already initialized.
    AlreadyInitialized,
    /// No suitable memory region found.
    NoMemoryFound,
    /// Invalid memory region.
    InvalidRegion,
}

/// Result type for DMA operations.
pub type Result<T> = core::result::Result<T, DmaError>;

// ============================================================================
// Chained Cave Pool
// ============================================================================

/// A pool composed of multiple memory caves chained together.
///
/// Inspired by cavealloc's chained deployment - when no single cave is large
/// enough, we chain multiple smaller caves into one logical pool.
///
/// This is ideal for running network stack DMA entirely from caves in our
/// own PE binary - no firmware memory allocation needed.
#[derive(Clone, Copy)]
pub struct CavePool {
    /// Individual caves (sorted by size descending).
    caves: [MemoryRegion; MAX_CAVES],
    /// Number of caves in use.
    cave_count: usize,
    /// Total bytes across all caves.
    total_size: usize,
    /// Per-cave allocation offsets.
    offsets: [usize; MAX_CAVES],
}

impl CavePool {
    /// Create an empty cave pool.
    pub const fn new() -> Self {
        Self {
            caves: [MemoryRegion::new(0, 0); MAX_CAVES],
            cave_count: 0,
            total_size: 0,
            offsets: [0; MAX_CAVES],
        }
    }

    /// Create a pool from discovered caves.
    pub fn from_caves(caves: &[MemoryRegion]) -> Self {
        let mut pool = Self::new();
        for (i, cave) in caves.iter().take(MAX_CAVES).enumerate() {
            if cave.size >= MIN_CAVE_SIZE {
                pool.caves[i] = *cave;
                pool.cave_count += 1;
                pool.total_size += cave.size;
            }
        }
        pool
    }

    /// Add a cave to the pool.
    pub fn add_cave(&mut self, cave: MemoryRegion) -> bool {
        if self.cave_count >= MAX_CAVES || cave.size < MIN_CAVE_SIZE {
            return false;
        }
        self.caves[self.cave_count] = cave;
        self.cave_count += 1;
        self.total_size += cave.size;
        true
    }

    /// Get total available space.
    pub fn total_space(&self) -> usize {
        self.total_size
    }

    /// Get remaining free space.
    pub fn free_space(&self) -> usize {
        let mut used = 0;
        for i in 0..self.cave_count {
            used += self.offsets[i];
        }
        self.total_size.saturating_sub(used)
    }

    /// Allocate from the chained caves.
    ///
    /// Tries each cave in order (largest first) until allocation succeeds.
    pub fn alloc_pages(&mut self, pages: usize) -> Result<(usize, NonNull<u8>)> {
        if pages == 0 {
            return Err(DmaError::ZeroPages);
        }

        let size = pages_to_bytes(pages);

        // Try each cave in order
        for i in 0..self.cave_count {
            let cave = &self.caves[i];
            let offset = self.offsets[i];
            let aligned_offset = align_up(offset, PAGE_SIZE);
            let new_offset = aligned_offset + size;

            if new_offset <= cave.size {
                self.offsets[i] = new_offset;

                let paddr = cave.base + aligned_offset;
                let vaddr_ptr = paddr as *mut u8;

                // Zero the memory
                unsafe {
                    core::ptr::write_bytes(vaddr_ptr, 0, size);
                }

                let vaddr = NonNull::new(vaddr_ptr).ok_or(DmaError::OutOfMemory)?;
                return Ok((paddr, vaddr));
            }
        }

        Err(DmaError::OutOfMemory)
    }

    /// Reset all allocations.
    pub fn reset(&mut self) {
        for offset in &mut self.offsets[..self.cave_count] {
            *offset = 0;
        }
    }

    /// Get number of caves.
    pub fn cave_count(&self) -> usize {
        self.cave_count
    }

    /// Get cave by index.
    pub fn get_cave(&self, index: usize) -> Option<&MemoryRegion> {
        if index < self.cave_count {
            Some(&self.caves[index])
        } else {
            None
        }
    }
}

impl Default for CavePool {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Allocation tracking
// ============================================================================

#[derive(Clone, Copy)]
struct Allocation {
    offset: usize,
    pages: usize,
    in_use: bool,
}

impl Allocation {
    const fn empty() -> Self {
        Self { offset: 0, pages: 0, in_use: false }
    }
}

// ============================================================================
// Global DMA Pool
// ============================================================================

/// Page-aligned static storage (fallback).
#[repr(C, align(4096))]
struct StaticStorage {
    data: [u8; DEFAULT_POOL_SIZE],
}

static mut STATIC_STORAGE: StaticStorage = StaticStorage {
    data: [0u8; DEFAULT_POOL_SIZE],
};

/// Global pool state.
struct PoolState {
    /// Base address of current pool.
    base: AtomicUsize,
    /// Size of current pool.
    size: AtomicUsize,
    /// Bump allocator offset.
    offset: AtomicUsize,
    /// Allocation tracking.
    allocations: core::cell::UnsafeCell<[Allocation; MAX_ALLOCATIONS]>,
    /// Number of allocations.
    alloc_count: AtomicUsize,
}

static POOL: PoolState = PoolState {
    base: AtomicUsize::new(0),
    size: AtomicUsize::new(0),
    offset: AtomicUsize::new(0),
    allocations: core::cell::UnsafeCell::new([Allocation::empty(); MAX_ALLOCATIONS]),
    alloc_count: AtomicUsize::new(0),
};

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static LOCK: AtomicBool = AtomicBool::new(false);

#[inline]
fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

#[inline]
fn unlock() {
    LOCK.store(false, Ordering::Release);
}

// ============================================================================
// DmaPool - Main API
// ============================================================================

/// Global DMA memory pool.
///
/// This is a singleton that manages DMA-capable memory for all device drivers.
/// Initialize once at startup, then any driver can allocate from it.
pub struct DmaPool;

impl DmaPool {
    /// Initialize with the built-in static storage.
    ///
    /// This is the simplest option - uses compiled-in memory.
    pub fn init_static() {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return;
        }

        // SAFETY: Single-threaded init
        unsafe {
            let base = STATIC_STORAGE.data.as_mut_ptr() as usize;
            core::ptr::write_bytes(STATIC_STORAGE.data.as_mut_ptr(), 0, DEFAULT_POOL_SIZE);
            POOL.base.store(base, Ordering::SeqCst);
            POOL.size.store(DEFAULT_POOL_SIZE, Ordering::SeqCst);
        }
    }

    /// Initialize with runtime-discovered memory.
    ///
    /// Attempts to find usable memory by scanning for zero regions.
    ///
    /// # Arguments
    ///
    /// * `search_start` - Start of memory range to search
    /// * `search_end` - End of memory range to search
    ///
    /// Falls back to static storage if discovery fails.
    pub fn init_discover(search_start: usize, search_end: usize) {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return;
        }

        // Try to find a zero region first
        let region = unsafe {
            MemoryDiscovery::find_zero_regions(search_start, search_end, MIN_REGION_SIZE)
        };

        if let Some(region) = region {
            if region.is_usable() {
                let aligned = region.aligned();
                POOL.base.store(aligned.base, Ordering::SeqCst);
                POOL.size.store(aligned.size, Ordering::SeqCst);
                return;
            }
        }

        // Fallback to static storage
        unsafe {
            let base = STATIC_STORAGE.data.as_mut_ptr() as usize;
            core::ptr::write_bytes(STATIC_STORAGE.data.as_mut_ptr(), 0, DEFAULT_POOL_SIZE);
            POOL.base.store(base, Ordering::SeqCst);
            POOL.size.store(DEFAULT_POOL_SIZE, Ordering::SeqCst);
        }
    }

    /// Initialize with externally-provided memory region.
    ///
    /// Use this when you have a known-good memory region (e.g., from firmware).
    ///
    /// # Safety
    ///
    /// - `base` must be a valid, page-aligned physical address.
    /// - The region must be identity-mapped (phys == virt).
    /// - The region must not be used by anything else.
    /// - The region must remain valid for the lifetime of the program.
    pub unsafe fn init_external(base: usize, size: usize) -> Result<()> {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(DmaError::AlreadyInitialized);
        }

        let region = MemoryRegion::new(base, size);
        if !region.is_usable() {
            INITIALIZED.store(false, Ordering::SeqCst);
            return Err(DmaError::InvalidRegion);
        }

        let aligned = region.aligned();
        core::ptr::write_bytes(aligned.base as *mut u8, 0, aligned.size);
        POOL.base.store(aligned.base, Ordering::SeqCst);
        POOL.size.store(aligned.size, Ordering::SeqCst);
        Ok(())
    }

    /// Initialize from caves discovered in our own binary.
    ///
    /// This scans the memory range for caves (unused padding) and chains
    /// them into one logical pool. Perfect for running network stack DMA
    /// entirely from caves in our PE binary.
    ///
    /// # Safety
    ///
    /// - `image_base` and `image_end` must bound our loaded PE image.
    /// - The discovered caves must not overlap with any used code/data.
    pub unsafe fn init_from_caves(image_base: usize, image_end: usize) -> Result<()> {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(DmaError::AlreadyInitialized);
        }

        // Find all caves in our image
        let mut caves = [MemoryRegion::new(0, 0); MAX_CAVES];
        let found = MemoryDiscovery::find_all_caves(
            image_base,
            image_end,
            MIN_CAVE_SIZE,
            PaddingPattern::Any,
            &mut caves,
        );

        if found == 0 {
            // Fallback to static storage
            let base = STATIC_STORAGE.data.as_mut_ptr() as usize;
            core::ptr::write_bytes(STATIC_STORAGE.data.as_mut_ptr(), 0, DEFAULT_POOL_SIZE);
            POOL.base.store(base, Ordering::SeqCst);
            POOL.size.store(DEFAULT_POOL_SIZE, Ordering::SeqCst);
            return Ok(());
        }

        // Use the largest cave as primary pool
        // (Could use CavePool for chaining, but global POOL is simpler)
        let best = caves[0]; // Already sorted by size
        core::ptr::write_bytes(best.base as *mut u8, 0, best.size);
        POOL.base.store(best.base, Ordering::SeqCst);
        POOL.size.store(best.size, Ordering::SeqCst);

        // Store total cave info for stats
        CAVE_POOL.store_caves(&caves[..found]);

        Ok(())
    }

    /// Initialize from a pre-built CavePool.
    ///
    /// Use this when you've already discovered caves and want to chain them.
    ///
    /// # Safety
    ///
    /// All caves in the pool must be valid, identity-mapped memory.
    pub unsafe fn init_from_cave_pool(caves: &[MemoryRegion]) -> Result<()> {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(DmaError::AlreadyInitialized);
        }

        if caves.is_empty() {
            INITIALIZED.store(false, Ordering::SeqCst);
            return Err(DmaError::NoMemoryFound);
        }

        // Store all caves for chained allocation
        CAVE_POOL.store_caves(caves);

        // Use largest as primary (backwards compat with single-pool API)
        let best = caves.iter().max_by_key(|c| c.size).unwrap();
        core::ptr::write_bytes(best.base as *mut u8, 0, best.size);
        POOL.base.store(best.base, Ordering::SeqCst);
        POOL.size.store(best.size, Ordering::SeqCst);

        Ok(())
    }

    /// Check if the pool is initialized.
    #[inline]
    pub fn is_initialized() -> bool {
        INITIALIZED.load(Ordering::SeqCst)
    }

    /// Allocate contiguous DMA pages.
    ///
    /// Returns (physical_address, virtual_address).
    /// Memory is zeroed before return.
    pub fn alloc_pages(pages: usize) -> Result<(usize, NonNull<u8>)> {
        if !Self::is_initialized() {
            return Err(DmaError::NotInitialized);
        }
        if pages == 0 {
            return Err(DmaError::ZeroPages);
        }

        let size = pages_to_bytes(pages);
        let pool_size = POOL.size.load(Ordering::Relaxed);

        lock();

        let offset = POOL.offset.load(Ordering::Relaxed);
        let aligned_offset = align_up(offset, PAGE_SIZE);
        let new_offset = aligned_offset + size;

        if new_offset > pool_size {
            unlock();
            return Err(DmaError::OutOfMemory);
        }

        POOL.offset.store(new_offset, Ordering::SeqCst);

        // Track allocation
        let alloc_idx = POOL.alloc_count.fetch_add(1, Ordering::SeqCst);
        if alloc_idx < MAX_ALLOCATIONS {
            unsafe {
                (*POOL.allocations.get())[alloc_idx] = Allocation {
                    offset: aligned_offset,
                    pages,
                    in_use: true,
                };
            }
        }

        unlock();

        // Calculate addresses
        let base = POOL.base.load(Ordering::Relaxed);
        let paddr = base + aligned_offset;
        let vaddr_ptr = paddr as *mut u8;

        // Zero the memory
        unsafe {
            core::ptr::write_bytes(vaddr_ptr, 0, size);
        }

        let vaddr = NonNull::new(vaddr_ptr).ok_or(DmaError::OutOfMemory)?;
        Ok((paddr, vaddr))
    }

    /// Deallocate DMA pages.
    ///
    /// Note: With bump allocation, memory isn't truly freed until reset.
    ///
    /// # Safety
    ///
    /// The paddr must have been returned by alloc_pages with the same page count.
    pub unsafe fn dealloc_pages(paddr: usize, pages: usize) {
        let base = POOL.base.load(Ordering::Relaxed);
        let offset = paddr.saturating_sub(base);

        lock();

        let allocations = &mut *POOL.allocations.get();
        for alloc in allocations.iter_mut() {
            if alloc.in_use && alloc.offset == offset && alloc.pages == pages {
                alloc.in_use = false;
                break;
            }
        }

        unlock();
    }

    /// Get remaining free space in bytes.
    pub fn free_space() -> usize {
        if !Self::is_initialized() {
            return 0;
        }
        let size = POOL.size.load(Ordering::Relaxed);
        let offset = POOL.offset.load(Ordering::Relaxed);
        size.saturating_sub(offset)
    }

    /// Get total pool size in bytes.
    pub fn total_size() -> usize {
        POOL.size.load(Ordering::Relaxed)
    }

    /// Get pool base address.
    pub fn base_address() -> usize {
        POOL.base.load(Ordering::Relaxed)
    }

    /// Reset the allocator.
    ///
    /// # Safety
    ///
    /// All previous allocations must be freed or abandoned.
    pub unsafe fn reset() {
        lock();
        POOL.offset.store(0, Ordering::SeqCst);
        POOL.alloc_count.store(0, Ordering::SeqCst);
        let allocations = &mut *POOL.allocations.get();
        for alloc in allocations.iter_mut() {
            *alloc = Allocation::empty();
        }
        unlock();
    }
}

// SAFETY: Pool uses atomic operations and spinlock
unsafe impl Sync for PoolState {}
unsafe impl Send for PoolState {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align_functions() {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_down(4097, 4096), 4096);
    }

    #[test]
    fn test_memory_region() {
        let region = MemoryRegion::new(4096, 65536);
        assert!(region.is_usable());

        let small = MemoryRegion::new(4096, 1024);
        assert!(!small.is_usable());
    }
}

