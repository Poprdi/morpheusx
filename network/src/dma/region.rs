//! DMA region definition and layout.
//!
//! # Memory Layout (2MB Region)
//! ```text
//! Offset      Size        Content
//! 0x00000     0x0200      RX Descriptor Table (32 × 16 bytes)
//! 0x00200     0x0048      RX Available Ring
//! 0x00400     0x0108      RX Used Ring
//! 0x00800     0x0200      TX Descriptor Table (32 × 16 bytes)
//! 0x00A00     0x0048      TX Available Ring
//! 0x00C00     0x0108      TX Used Ring
//! 0x01000     0x10000     RX Buffers (32 × 2KB)
//! 0x11000     0x10000     TX Buffers (32 × 2KB)
//! ```
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.3

/// DMA-capable memory region.
///
/// Contains both the CPU-accessible pointer and the device-visible bus address.
pub struct DmaRegion {
    /// CPU-accessible pointer to the region.
    pub cpu_ptr: *mut u8,
    /// Device-visible bus address.
    pub bus_addr: u64,
    /// Total size of the region in bytes.
    pub size: usize,
}

impl DmaRegion {
    /// Minimum region size (2MB).
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;
    
    /// Default queue size (number of descriptors).
    pub const DEFAULT_QUEUE_SIZE: usize = 32;
    
    /// Default buffer size (2KB each).
    pub const DEFAULT_BUFFER_SIZE: usize = 2048;
    
    // Layout offsets
    
    /// RX descriptor table offset.
    pub const RX_DESC_OFFSET: usize = 0x0000;
    /// RX available ring offset.
    pub const RX_AVAIL_OFFSET: usize = 0x0200;
    /// RX used ring offset.
    pub const RX_USED_OFFSET: usize = 0x0400;
    /// TX descriptor table offset.
    pub const TX_DESC_OFFSET: usize = 0x0800;
    /// TX available ring offset.
    pub const TX_AVAIL_OFFSET: usize = 0x0A00;
    /// TX used ring offset.
    pub const TX_USED_OFFSET: usize = 0x0C00;
    /// RX buffers offset.
    pub const RX_BUFFERS_OFFSET: usize = 0x1000;
    /// TX buffers offset.
    pub const TX_BUFFERS_OFFSET: usize = 0x11000;
    
    /// Create a new DMA region.
    ///
    /// # Safety
    /// - `cpu_ptr` must point to valid DMA-capable memory
    /// - `bus_addr` must be the corresponding device-visible address
    /// - Region must be properly aligned (page-aligned preferred)
    pub unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self {
        debug_assert!(size >= Self::MIN_SIZE, "DMA region too small");
        Self { cpu_ptr, bus_addr, size }
    }
    
    /// Get CPU pointer for RX descriptor table.
    pub fn rx_desc_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_DESC_OFFSET) }
    }
    
    /// Get bus address for RX descriptor table.
    pub fn rx_desc_bus(&self) -> u64 {
        self.bus_addr + Self::RX_DESC_OFFSET as u64
    }
    
    /// Get CPU pointer for RX available ring.
    pub fn rx_avail_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_AVAIL_OFFSET) }
    }
    
    /// Get bus address for RX available ring.
    pub fn rx_avail_bus(&self) -> u64 {
        self.bus_addr + Self::RX_AVAIL_OFFSET as u64
    }
    
    /// Get CPU pointer for RX used ring.
    pub fn rx_used_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_USED_OFFSET) }
    }
    
    /// Get bus address for RX used ring.
    pub fn rx_used_bus(&self) -> u64 {
        self.bus_addr + Self::RX_USED_OFFSET as u64
    }
    
    /// Get CPU pointer for TX descriptor table.
    pub fn tx_desc_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_DESC_OFFSET) }
    }
    
    /// Get bus address for TX descriptor table.
    pub fn tx_desc_bus(&self) -> u64 {
        self.bus_addr + Self::TX_DESC_OFFSET as u64
    }
    
    /// Get CPU pointer for TX available ring.
    pub fn tx_avail_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_AVAIL_OFFSET) }
    }
    
    /// Get bus address for TX available ring.
    pub fn tx_avail_bus(&self) -> u64 {
        self.bus_addr + Self::TX_AVAIL_OFFSET as u64
    }
    
    /// Get CPU pointer for TX used ring.
    pub fn tx_used_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_USED_OFFSET) }
    }
    
    /// Get bus address for TX used ring.
    pub fn tx_used_bus(&self) -> u64 {
        self.bus_addr + Self::TX_USED_OFFSET as u64
    }
    
    /// Get CPU pointer for RX buffers.
    pub fn rx_buffers_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_BUFFERS_OFFSET) }
    }
    
    /// Get bus address for RX buffers.
    pub fn rx_buffers_bus(&self) -> u64 {
        self.bus_addr + Self::RX_BUFFERS_OFFSET as u64
    }
    
    /// Get CPU pointer for TX buffers.
    pub fn tx_buffers_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_BUFFERS_OFFSET) }
    }
    
    /// Get bus address for TX buffers.
    pub fn tx_buffers_bus(&self) -> u64 {
        self.bus_addr + Self::TX_BUFFERS_OFFSET as u64
    }
    
    /// Calculate buffer address by index.
    pub fn buffer_cpu(&self, offset: usize, index: usize, buffer_size: usize) -> *mut u8 {
        unsafe { self.cpu_ptr.add(offset + index * buffer_size) }
    }
    
    /// Calculate buffer bus address by index.
    pub fn buffer_bus(&self, offset: usize, index: usize, buffer_size: usize) -> u64 {
        self.bus_addr + (offset + index * buffer_size) as u64
    }
}

unsafe impl Send for DmaRegion {}
unsafe impl Sync for DmaRegion {}
