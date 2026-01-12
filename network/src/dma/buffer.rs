//! DMA buffer with ownership tracking.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.5

use super::ownership::BufferOwnership;

/// A single DMA buffer with ownership tracking.
///
/// Tracks both CPU and bus addresses, plus ownership state.
pub struct DmaBuffer {
    /// CPU-accessible pointer to buffer data.
    cpu_ptr: *mut u8,
    /// Device-visible bus address.
    bus_addr: u64,
    /// Buffer capacity in bytes.
    capacity: usize,
    /// Current ownership state.
    ownership: BufferOwnership,
    /// Buffer index within the pool.
    index: u16,
}

impl DmaBuffer {
    /// Create a new DMA buffer.
    ///
    /// # Safety
    /// - `cpu_ptr` must point to valid DMA-capable memory
    /// - `bus_addr` must be the corresponding device-visible address
    pub unsafe fn new(cpu_ptr: *mut u8, bus_addr: u64, capacity: usize, index: u16) -> Self {
        Self {
            cpu_ptr,
            bus_addr,
            capacity,
            ownership: BufferOwnership::Free,
            index,
        }
    }

    /// Get buffer data as slice.
    ///
    /// # Panics
    /// Panics if buffer is not DriverOwned.
    pub fn as_slice(&self) -> &[u8] {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "BUG: Cannot access buffer not owned by driver (state: {:?})",
            self.ownership
        );
        unsafe { core::slice::from_raw_parts(self.cpu_ptr, self.capacity) }
    }

    /// Get buffer data as mutable slice.
    ///
    /// # Panics
    /// Panics if buffer is not DriverOwned.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "BUG: Cannot access buffer not owned by driver (state: {:?})",
            self.ownership
        );
        unsafe { core::slice::from_raw_parts_mut(self.cpu_ptr, self.capacity) }
    }

    /// Get the first `len` bytes as mutable slice.
    ///
    /// # Panics
    /// Panics if buffer is not DriverOwned or len > capacity.
    pub fn as_mut_slice_len(&mut self, len: usize) -> &mut [u8] {
        assert!(
            len <= self.capacity,
            "Requested length exceeds buffer capacity"
        );
        &mut self.as_mut_slice()[..len]
    }

    /// Get the device-visible bus address.
    pub fn bus_addr(&self) -> u64 {
        self.bus_addr
    }

    /// Get the CPU pointer.
    pub fn cpu_ptr(&self) -> *mut u8 {
        self.cpu_ptr
    }

    /// Get buffer index.
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Get buffer capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get current ownership state.
    pub fn ownership(&self) -> BufferOwnership {
        self.ownership
    }

    /// Check if buffer can be allocated.
    pub fn is_free(&self) -> bool {
        self.ownership.is_free()
    }

    /// Check if buffer is owned by driver.
    pub fn is_driver_owned(&self) -> bool {
        self.ownership.can_access()
    }

    /// Check if buffer is owned by device.
    pub fn is_device_owned(&self) -> bool {
        self.ownership.is_device_owned()
    }

    /// Mark buffer as allocated (Free -> DriverOwned).
    ///
    /// # Safety
    /// Only call during allocation from pool.
    pub(crate) unsafe fn mark_allocated(&mut self) {
        assert!(self.ownership.is_free(), "Buffer must be free to allocate");
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// Mark buffer as device-owned (DriverOwned -> DeviceOwned).
    ///
    /// # Safety
    /// Only call immediately before submitting to device.
    pub unsafe fn mark_device_owned(&mut self) {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "Buffer must be driver-owned before device transfer"
        );
        self.ownership = BufferOwnership::DeviceOwned;
    }

    /// Mark buffer as driver-owned (DeviceOwned -> DriverOwned).
    ///
    /// # Safety
    /// Only call after device confirms ownership transfer (poll completion).
    pub unsafe fn mark_driver_owned(&mut self) {
        assert!(
            self.ownership == BufferOwnership::DeviceOwned,
            "Buffer must be device-owned before reclaim"
        );
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// Force buffer to driver-owned state for error recovery.
    ///
    /// This bypasses normal state machine validation for cases where
    /// a device operation failed and we need to reclaim the buffer
    /// regardless of its current state (e.g., submit failed after
    /// marking device-owned).
    pub fn force_driver_owned(&mut self) {
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// Mark buffer as free (DriverOwned -> Free).
    ///
    /// # Safety
    /// Only call during return to pool.
    pub(crate) unsafe fn mark_free(&mut self) {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "Buffer must be driver-owned before freeing"
        );
        self.ownership = BufferOwnership::Free;
    }
}

unsafe impl Send for DmaBuffer {}
unsafe impl Sync for DmaBuffer {}
