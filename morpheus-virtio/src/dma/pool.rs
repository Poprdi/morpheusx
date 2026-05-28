//! Pre-allocated DMA buffer pool with free-list tracking.

use super::buffer::DmaBuffer;

pub const MAX_POOL_SIZE: usize = 32;

pub struct BufferPool {
    buffers: [Option<DmaBuffer>; MAX_POOL_SIZE],
    free_list: [u16; MAX_POOL_SIZE],
    free_count: usize,
    total_count: usize,
    buffer_size: usize,
}

impl BufferPool {
    /// # Safety
    /// - `cpu_base` must point to valid DMA-capable memory
    /// - `bus_base` must be the corresponding device-visible address
    /// - Region must hold at least `buffer_size * count` bytes
    pub unsafe fn new(cpu_base: *mut u8, bus_base: u64, buffer_size: usize, count: usize) -> Self {
        assert!(count <= MAX_POOL_SIZE, "Pool size exceeds maximum");
        assert!(buffer_size > 0, "Buffer size must be positive");

        let mut buffers: [Option<DmaBuffer>; MAX_POOL_SIZE] = Default::default();
        let mut free_list = [0u16; MAX_POOL_SIZE];

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

    /// Returns `None` if the pool is exhausted.
    pub fn alloc(&mut self) -> Option<&mut DmaBuffer> {
        if self.free_count == 0 {
            return None;
        }

        self.free_count -= 1;
        let idx = self.free_list[self.free_count] as usize;

        let buf = self.buffers[idx].as_mut()?;
        debug_assert!(buf.is_free(), "Allocated buffer must be free");
        unsafe {
            buf.mark_allocated();
        }

        Some(buf)
    }

    pub fn free(&mut self, index: u16) {
        let idx = index as usize;
        assert!(idx < self.total_count, "Invalid buffer index");

        if let Some(buf) = self.buffers[idx].as_mut() {
            // Driver-owned check rejects double-free that would corrupt free_list.
            assert!(
                buf.is_driver_owned(),
                "Can only free driver-owned buffers (double-free detected)"
            );
            unsafe {
                buf.mark_free();
            }

            assert!(
                self.free_count < self.total_count,
                "Free count overflow (pool corruption)"
            );
            self.free_list[self.free_count] = index;
            self.free_count += 1;
        }
    }

    pub fn get_mut(&mut self, index: u16) -> Option<&mut DmaBuffer> {
        let idx = index as usize;
        if idx >= self.total_count {
            return None;
        }
        self.buffers[idx].as_mut()
    }

    pub fn get(&self, index: u16) -> Option<&DmaBuffer> {
        let idx = index as usize;
        if idx >= self.total_count {
            return None;
        }
        self.buffers[idx].as_ref()
    }

    pub fn available(&self) -> usize {
        self.free_count
    }

    pub fn total(&self) -> usize {
        self.total_count
    }

    pub fn in_use(&self) -> usize {
        self.total_count - self.free_count
    }

    pub fn is_empty(&self) -> bool {
        self.free_count == 0
    }

    pub fn is_full(&self) -> bool {
        self.free_count == self.total_count
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    pub fn iter(&self) -> impl Iterator<Item = &DmaBuffer> {
        self.buffers.iter().filter_map(|b| b.as_ref())
    }

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
