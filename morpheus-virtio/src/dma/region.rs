//! 2 MB DMA region for one VirtIO net device.
//!
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

pub struct DmaRegion {
    pub cpu_ptr: *mut u8,
    pub bus_addr: u64,
    pub size: usize,
}

impl DmaRegion {
    pub const MIN_SIZE: usize = 2 * 1024 * 1024;

    pub const DEFAULT_QUEUE_SIZE: usize = 32;

    pub const DEFAULT_BUFFER_SIZE: usize = 2048;

    pub const RX_DESC_OFFSET: usize = 0x0000;
    pub const RX_AVAIL_OFFSET: usize = 0x0200;
    pub const RX_USED_OFFSET: usize = 0x0400;
    pub const TX_DESC_OFFSET: usize = 0x0800;
    pub const TX_AVAIL_OFFSET: usize = 0x0A00;
    pub const TX_USED_OFFSET: usize = 0x0C00;
    pub const RX_BUFFERS_OFFSET: usize = 0x1000;
    pub const TX_BUFFERS_OFFSET: usize = 0x11000;

    /// # Safety
    /// - `cpu_ptr` must point to valid DMA-capable memory
    /// - `bus_addr` must be the corresponding device-visible address
    /// - Region must be page-aligned
    pub unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, size: usize) -> Self {
        debug_assert!(size >= Self::MIN_SIZE, "DMA region too small");
        Self {
            cpu_ptr,
            bus_addr,
            size,
        }
    }

    pub fn cpu_base(&self) -> *mut u8 {
        self.cpu_ptr
    }

    pub fn bus_base(&self) -> u64 {
        self.bus_addr
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn rx_desc_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_DESC_OFFSET) }
    }

    pub fn rx_desc_bus(&self) -> u64 {
        self.bus_addr + Self::RX_DESC_OFFSET as u64
    }

    pub fn rx_avail_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_AVAIL_OFFSET) }
    }

    pub fn rx_avail_bus(&self) -> u64 {
        self.bus_addr + Self::RX_AVAIL_OFFSET as u64
    }

    pub fn rx_used_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_USED_OFFSET) }
    }

    pub fn rx_used_bus(&self) -> u64 {
        self.bus_addr + Self::RX_USED_OFFSET as u64
    }

    pub fn tx_desc_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_DESC_OFFSET) }
    }

    pub fn tx_desc_bus(&self) -> u64 {
        self.bus_addr + Self::TX_DESC_OFFSET as u64
    }

    pub fn tx_avail_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_AVAIL_OFFSET) }
    }

    pub fn tx_avail_bus(&self) -> u64 {
        self.bus_addr + Self::TX_AVAIL_OFFSET as u64
    }

    pub fn tx_used_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_USED_OFFSET) }
    }

    pub fn tx_used_bus(&self) -> u64 {
        self.bus_addr + Self::TX_USED_OFFSET as u64
    }

    pub fn rx_buffers_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::RX_BUFFERS_OFFSET) }
    }

    pub fn rx_buffers_bus(&self) -> u64 {
        self.bus_addr + Self::RX_BUFFERS_OFFSET as u64
    }

    pub fn tx_buffers_cpu(&self) -> *mut u8 {
        unsafe { self.cpu_ptr.add(Self::TX_BUFFERS_OFFSET) }
    }

    pub fn tx_buffers_bus(&self) -> u64 {
        self.bus_addr + Self::TX_BUFFERS_OFFSET as u64
    }

    pub fn buffer_cpu(&self, offset: usize, index: usize, buffer_size: usize) -> *mut u8 {
        unsafe { self.cpu_ptr.add(offset + index * buffer_size) }
    }

    pub fn buffer_bus(&self, offset: usize, index: usize, buffer_size: usize) -> u64 {
        self.bus_addr + (offset + index * buffer_size) as u64
    }
}

unsafe impl Send for DmaRegion {}
unsafe impl Sync for DmaRegion {}
