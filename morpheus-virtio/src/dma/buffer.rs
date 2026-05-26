//! DMA buffer with ownership tracking.

use super::ownership::BufferOwnership;

pub struct DmaBuffer {
    cpu_ptr: *mut u8,
    bus_addr: u64,
    capacity: usize,
    ownership: BufferOwnership,
    index: u16,
}

impl DmaBuffer {
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

    pub fn as_slice(&self) -> &[u8] {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "BUG: Cannot access buffer not owned by driver (state: {:?})",
            self.ownership
        );
        unsafe { core::slice::from_raw_parts(self.cpu_ptr, self.capacity) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "BUG: Cannot access buffer not owned by driver (state: {:?})",
            self.ownership
        );
        unsafe { core::slice::from_raw_parts_mut(self.cpu_ptr, self.capacity) }
    }

    pub fn as_mut_slice_len(&mut self, len: usize) -> &mut [u8] {
        assert!(
            len <= self.capacity,
            "Requested length exceeds buffer capacity"
        );
        &mut self.as_mut_slice()[..len]
    }

    pub fn bus_addr(&self) -> u64 {
        self.bus_addr
    }

    pub fn cpu_ptr(&self) -> *mut u8 {
        self.cpu_ptr
    }

    pub fn index(&self) -> u16 {
        self.index
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn ownership(&self) -> BufferOwnership {
        self.ownership
    }

    pub fn is_free(&self) -> bool {
        self.ownership.is_free()
    }

    pub fn is_driver_owned(&self) -> bool {
        self.ownership.can_access()
    }

    pub fn is_device_owned(&self) -> bool {
        self.ownership.is_device_owned()
    }

    /// Free -> DriverOwned. Pool alloc only.
    pub(crate) unsafe fn mark_allocated(&mut self) {
        assert!(self.ownership.is_free(), "Buffer must be free to allocate");
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// DriverOwned -> DeviceOwned. Call immediately before submit.
    pub unsafe fn mark_device_owned(&mut self) {
        assert!(
            self.ownership == BufferOwnership::DriverOwned,
            "Buffer must be driver-owned before device transfer"
        );
        self.ownership = BufferOwnership::DeviceOwned;
    }

    /// DeviceOwned -> DriverOwned. Call after observed completion.
    pub unsafe fn mark_driver_owned(&mut self) {
        assert!(
            self.ownership == BufferOwnership::DeviceOwned,
            "Buffer must be device-owned before reclaim"
        );
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// Bypass state machine for error recovery (e.g. submit failed after
    /// the buffer was already marked device-owned).
    pub fn force_driver_owned(&mut self) {
        self.ownership = BufferOwnership::DriverOwned;
    }

    /// DriverOwned -> Free. Pool return only.
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
