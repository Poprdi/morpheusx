//! Buffer pool management.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.5

use super::buffer::DmaBuffer;
use super::ownership::BufferOwnership;

/// Maximum number of buffers per pool.
pub const MAX_POOL_SIZE: usize = 32;

/// Pre-allocated buffer pool for a virtqueue.
///
/// Manages a fixed set of DMA buffers with free list tracking.
pub struct BufferPool {
    /// Array of DMA buffers.
    buffers: [Option<DmaBuffer>; MAX_POOL_SIZE],
    /// Free list (indices of free buffers).
    free_list: [u16; MAX_POOL_SIZE],
    /// Number of free buffers.
    free_count: usize,
    /// Total number of buffers in pool.
    total_count: usize,
    /// Size of each buffer.
    buffer_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool from DMA region.
    ///
    /// # Arguments
    /// - `cpu_base`: CPU pointer to buffer region start
    /// - `bus_base`: Bus address of buffer region start
    /// - `buffer_size`: Size of each buffer (should be >= 1526 for RX)
    /// - `count`: Number of buffers (max 32)
    ///
    /// # Safety
    /// - `cpu_base` must point to valid DMA-capable memory
    /// - `bus_base` must be the corresponding device-visible address
    /// - Memory must be at least `buffer_size * count` bytes
    pub unsafe fn new(
        cpu_base: *mut u8,
        bus_base: u64,
        buffer_size: usize,
        count: usize,
    ) -> Self {
        assert!(count <= MAX_POOL_SIZE, "Pool size exceeds maximum");
        assert!(buffer_size > 0, "Buffer size must be positive");
        
        // Initialize buffers array with None
        let mut buffers: [Option<DmaBuffer>; MAX_POOL_SIZE] = Default::default();
        let mut free_list = [0u16; MAX_POOL_SIZE];
        
        // Create buffers
        for i in 0..count {
            let cpu_ptr = cpu_base.add(i * buffer_size);
            let bus_addr = bus_base + (i * buffer_size) as u64;
            buffers[i] = Some(DmaBuffer::new(cpu_ptr, bus_addr, buffer_size, i as u16));
            free_list[i] = i as u16;
        }
        
        Self {
            buffers,
            free_list,
            free_count: count,
            total_count: count,
            buffer_size,
        }
    }
    
    /// Allocate a buffer from the pool.
    ///
    /// Returns `None` if pool is exhausted.
    pub fn alloc(&mut self) -> Option<&mut DmaBuffer> {
        if self.free_count == 0 {
            return None;
        }
        
        self.free_count -= 1;
        let idx = self.free_list[self.free_count] as usize;
        
        let buf = self.buffers[idx].as_mut()?;
        debug_assert!(buf.is_free(), "Allocated buffer must be free");
        unsafe { buf.mark_allocated(); }
        
        Some(buf)
    }
    
    /// Return a buffer to the pool.
    ///
    /// # Arguments
    /// - `index`: Buffer index (must be valid and driver-owned)
    pub fn free(&mut self, index: u16) {
        let idx = index as usize;
        assert!(idx < self.total_count, "Invalid buffer index");
        
        if let Some(buf) = self.buffers[idx].as_mut() {
            debug_assert!(buf.is_driver_owned(), "Can only free driver-owned buffers");
            unsafe { buf.mark_free(); }
            
            self.free_list[self.free_count] = index;
            self.free_count += 1;
        }
    }
    
    /// Get mutable reference to buffer by index.
    ///
    /// # Panics
    /// Panics if index is out of range.
    pub fn get_mut(&mut self, index: u16) -> Option<&mut DmaBuffer> {
        let idx = index as usize;
        if idx >= self.total_count {
            return None;
        }
        self.buffers[idx].as_mut()
    }
    
    /// Get reference to buffer by index.
    pub fn get(&self, index: u16) -> Option<&DmaBuffer> {
        let idx = index as usize;
        if idx >= self.total_count {
            return None;
        }
        self.buffers[idx].as_ref()
    }
    
    /// Get number of available (free) buffers.
    pub fn available(&self) -> usize {
        self.free_count
    }
    
    /// Get total number of buffers in pool.
    pub fn total(&self) -> usize {
        self.total_count
    }
    
    /// Get number of buffers currently in use.
    pub fn in_use(&self) -> usize {
        self.total_count - self.free_count
    }
    
    /// Check if pool is empty (no free buffers).
    pub fn is_empty(&self) -> bool {
        self.free_count == 0
    }
    
    /// Check if pool is full (all buffers free).
    pub fn is_full(&self) -> bool {
        self.free_count == self.total_count
    }
    
    /// Get buffer size.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
    
    /// Iterate over all buffers (for debugging).
    pub fn iter(&self) -> impl Iterator<Item = &DmaBuffer> {
        self.buffers.iter().filter_map(|b| b.as_ref())
    }
    
    /// Iterate over all buffers mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut DmaBuffer> {
        self.buffers.iter_mut().filter_map(|b| b.as_mut())
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self {
            buffers: Default::default(),
            free_list: [0; MAX_POOL_SIZE],
            free_count: 0,
            total_count: 0,
            buffer_size: 0,
        }
    }
}
