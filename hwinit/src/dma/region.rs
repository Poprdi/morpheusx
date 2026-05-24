//! Generic DMA-capable region. Layout is the driver's problem.

/// CPU pointer + device-visible bus address pair.
#[derive(Clone, Copy)]
pub struct DmaRegion {
    cpu_ptr: *mut u8,
    bus_addr: u64,
    size: usize,
}

impl DmaRegion {
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;

    /// # Safety
    /// `cpu_ptr` and `bus_addr` must address the same DMA-capable memory;
    /// caller ensures identity-mapping or IOMMU programming.
    pub const unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self {
        Self {
            cpu_ptr,
            bus_addr,
            size,
        }
    }

    #[inline]
    pub const fn cpu_base(&self) -> *mut u8 {
        self.cpu_ptr
    }

    #[inline]
    pub const fn bus_base(&self) -> u64 {
        self.bus_addr
    }

    #[inline]
    pub const fn size(&self) -> usize {
        self.size
    }

    /// # Safety
    /// `offset` must be within bounds.
    #[inline]
    pub unsafe fn cpu_at(&self, offset: usize) -> *mut u8 {
        self.cpu_ptr.add(offset)
    }

    #[inline]
    pub const fn bus_at(&self, offset: usize) -> u64 {
        self.bus_addr + offset as u64
    }

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
