//! DMA region abstraction.
//!
//! Generic DMA-capable memory region. Layout-specific offsets belong in drivers.

/// DMA-capable memory region.
///
/// Contains both CPU-accessible pointer and device-visible bus address.
/// Drivers are responsible for their own layout within this region.
#[derive(Clone, Copy)]
pub struct DmaRegion {
    cpu_ptr: *mut u8,
    bus_addr: u64,
    size: usize,
}

impl DmaRegion {
    /// Minimum region size (2MB).
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;

    /// Create a new DMA region.
    ///
    /// # Safety
    /// - `cpu_ptr` must point to valid DMA-capable memory
    /// - `bus_addr` must be the corresponding device-visible address
    /// - Memory must be identity-mapped or IOMMU configured
    pub const unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self {
        Self { cpu_ptr, bus_addr, size }
    }

    /// CPU base pointer.
    #[inline]
    pub const fn cpu_base(&self) -> *mut u8 {
        self.cpu_ptr
    }

    /// Bus base address (what devices see).
    #[inline]
    pub const fn bus_base(&self) -> u64 {
        self.bus_addr
    }

    /// Total size in bytes.
    #[inline]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// Get CPU pointer at offset.
    ///
    /// # Safety
    /// Offset must be within region bounds.
    #[inline]
    pub unsafe fn cpu_at(&self, offset: usize) -> *mut u8 {
        self.cpu_ptr.add(offset)
    }

    /// Get bus address at offset.
    #[inline]
    pub const fn bus_at(&self, offset: usize) -> u64 {
        self.bus_addr + offset as u64
    }

    /// Check if region is valid (non-null, reasonable size).
    pub fn is_valid(&self) -> bool {
        !self.cpu_ptr.is_null() && self.size >= Self::MIN_SIZE
    }
}

unsafe impl Send for DmaRegion {}
unsafe impl Sync for DmaRegion {}

impl core::fmt::Debug for DmaRegion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DmaRegion")
            .field("cpu_ptr", &self.cpu_ptr)
            .field("bus_addr", &format_args!("{:#x}", self.bus_addr))
            .field("size", &format_args!("{:#x}", self.size))
            .finish()
    }
}
