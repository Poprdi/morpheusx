//! VirtIO ASM bindings.
//!
//! Complete bindings for VirtIO device initialization and virtqueue operations.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.2, §4

use crate::types::repr_c::{VirtqueueState, RxResult};

// ═══════════════════════════════════════════════════════════════════════════
// Device Initialization Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Verify VirtIO magic value (0x74726976 = "virt").
    /// Returns 1 if valid, 0 if not.
    fn asm_virtio_verify_magic(mmio_base: u64) -> u32;
    
    /// Get VirtIO device version (2 = modern).
    fn asm_virtio_get_version(mmio_base: u64) -> u32;
    
    /// Get VirtIO device ID (1 = net, 2 = block, etc.).
    fn asm_virtio_get_device_id(mmio_base: u64) -> u32;
    
    /// Reset VirtIO device (write 0 to status, wait for completion).
    /// Returns 0 on success, 1 on timeout.
    fn asm_virtio_reset(mmio_base: u64) -> u32;
    
    /// Set VirtIO device status.
    fn asm_virtio_set_status(mmio_base: u64, status: u8);
    
    /// Get VirtIO device status.
    fn asm_virtio_get_status(mmio_base: u64) -> u8;
    
    /// Read device feature bits (64-bit, handles feature_sel).
    fn asm_virtio_read_features(mmio_base: u64) -> u64;
    
    /// Write driver-accepted feature bits.
    fn asm_virtio_write_features(mmio_base: u64, features: u64);
    
    /// Read MAC address from config space.
    /// Returns 0 on success, 1 if MAC feature not negotiated.
    fn asm_virtio_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// Virtqueue Configuration Functions
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Select virtqueue by index.
    fn asm_vq_select(mmio_base: u64, queue_idx: u16);
    
    /// Get maximum queue size for selected queue.
    fn asm_vq_get_max_size(mmio_base: u64) -> u16;
    
    /// Set queue size for selected queue.
    fn asm_vq_set_size(mmio_base: u64, size: u16);
    
    /// Set descriptor table address.
    fn asm_vq_set_desc(mmio_base: u64, addr: u64);
    
    /// Set driver (available) ring address.
    fn asm_vq_set_driver(mmio_base: u64, addr: u64);
    
    /// Set device (used) ring address.
    fn asm_vq_set_device(mmio_base: u64, addr: u64);
    
    /// Enable selected queue.
    fn asm_vq_enable(mmio_base: u64);
    
    /// Disable selected queue.
    fn asm_vq_disable(mmio_base: u64);
    
    /// Check if queue is ready.
    fn asm_vq_is_ready(mmio_base: u64) -> u32;
    
    /// Full queue setup helper.
    fn asm_vq_setup(mmio_base: u64, queue_idx: u16, size: u16, 
                   desc_addr: u64, driver_addr: u64, device_addr: u64);
    
    /// Initialize a single descriptor.
    fn asm_vq_init_desc(desc_table: u64, idx: u16, addr: u64, len: u32, flags: u16, next: u16);
    
    /// Initialize descriptor chain (for multi-descriptor buffers).
    fn asm_vq_init_desc_chain(desc_table: u64, start_idx: u16, addrs: *const u64, 
                              lens: *const u32, count: u16, write_flags: u16);
}

// ═══════════════════════════════════════════════════════════════════════════
// TX/RX Operations
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Submit buffer to TX queue.
    /// Returns 0 on success, 1 if queue full.
    fn asm_vq_submit_tx(vq: *mut VirtqueueState, buffer_idx: u16, buffer_len: u16) -> u32;
    
    /// Poll TX completion.
    /// Returns buffer index (0-0xFFFE) or 0xFFFFFFFF if no completion.
    fn asm_vq_poll_tx_complete(vq: *mut VirtqueueState) -> u32;
    
    /// Get number of available TX slots.
    fn asm_vq_tx_avail_slots(vq: *mut VirtqueueState) -> u16;
    
    /// Submit buffer to RX queue.
    /// Returns 0 on success, 1 if queue full.
    fn asm_vq_submit_rx(vq: *mut VirtqueueState, buffer_idx: u16, capacity: u16) -> u32;
    
    /// Poll for received packet.
    /// Returns 0 if no packet, 1 if packet ready (result populated).
    fn asm_vq_poll_rx(vq: *mut VirtqueueState, result: *mut RxResult) -> u32;
    
    /// Get number of pending RX packets.
    fn asm_vq_rx_pending(vq: *mut VirtqueueState) -> u16;
}

// ═══════════════════════════════════════════════════════════════════════════
// Queue Notification
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
extern "win64" {
    /// Notify device about queue activity (uses vq.notify_addr).
    fn asm_vq_notify(vq: *mut VirtqueueState);
    
    /// Direct notify with explicit address.
    fn asm_vq_notify_direct(notify_addr: u64, queue_idx: u16);
    
    /// Check if notification is needed (event suppression).
    fn asm_vq_should_notify(vq: *mut VirtqueueState) -> u32;
    
    /// Set notify address in VirtqueueState.
    fn asm_vq_set_notify_addr(vq: *mut VirtqueueState, addr: u64);
}

// ═══════════════════════════════════════════════════════════════════════════
// Safe Rust Wrappers
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO device operations.
pub mod device {
    use super::*;

    /// Check if MMIO region contains valid VirtIO device.
    #[cfg(target_arch = "x86_64")]
    pub fn verify_magic(mmio_base: u64) -> bool {
        unsafe { asm_virtio_verify_magic(mmio_base) == 1 }
    }
    
    /// Get device version (2 = modern VirtIO 1.0+).
    #[cfg(target_arch = "x86_64")]
    pub fn get_version(mmio_base: u64) -> u32 {
        unsafe { asm_virtio_get_version(mmio_base) }
    }
    
    /// Get device type ID (1 = network).
    #[cfg(target_arch = "x86_64")]
    pub fn get_device_id(mmio_base: u64) -> u32 {
        unsafe { asm_virtio_get_device_id(mmio_base) }
    }
    
    /// Reset device. Returns true on success.
    #[cfg(target_arch = "x86_64")]
    pub fn reset(mmio_base: u64) -> bool {
        unsafe { asm_virtio_reset(mmio_base) == 0 }
    }
    
    /// Set device status bits.
    #[cfg(target_arch = "x86_64")]
    pub fn set_status(mmio_base: u64, status: u8) {
        unsafe { asm_virtio_set_status(mmio_base, status) }
    }
    
    /// Get current device status.
    #[cfg(target_arch = "x86_64")]
    pub fn get_status(mmio_base: u64) -> u8 {
        unsafe { asm_virtio_get_status(mmio_base) }
    }
    
    /// Read device feature bits.
    #[cfg(target_arch = "x86_64")]
    pub fn read_features(mmio_base: u64) -> u64 {
        unsafe { asm_virtio_read_features(mmio_base) }
    }
    
    /// Write driver-accepted feature bits.
    #[cfg(target_arch = "x86_64")]
    pub fn write_features(mmio_base: u64, features: u64) {
        unsafe { asm_virtio_write_features(mmio_base, features) }
    }
    
    /// Read MAC address. Returns None if not available.
    #[cfg(target_arch = "x86_64")]
    pub fn read_mac(mmio_base: u64) -> Option<[u8; 6]> {
        let mut mac = [0u8; 6];
        if unsafe { asm_virtio_read_mac(mmio_base, &mut mac) } == 0 {
            Some(mac)
        } else {
            None
        }
    }
    
    // Stubs for non-x86_64
    #[cfg(not(target_arch = "x86_64"))]
    pub fn verify_magic(_mmio_base: u64) -> bool { false }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn get_version(_mmio_base: u64) -> u32 { 0 }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn get_device_id(_mmio_base: u64) -> u32 { 0 }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn reset(_mmio_base: u64) -> bool { false }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn set_status(_mmio_base: u64, _status: u8) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn get_status(_mmio_base: u64) -> u8 { 0 }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn read_features(_mmio_base: u64) -> u64 { 0 }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn write_features(_mmio_base: u64, _features: u64) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn read_mac(_mmio_base: u64) -> Option<[u8; 6]> { None }
}

/// Virtqueue operations.
pub mod queue {
    use super::*;

    /// Select a virtqueue by index.
    #[cfg(target_arch = "x86_64")]
    pub fn select(mmio_base: u64, queue_idx: u16) {
        unsafe { asm_vq_select(mmio_base, queue_idx) }
    }
    
    /// Get max queue size for selected queue.
    #[cfg(target_arch = "x86_64")]
    pub fn get_max_size(mmio_base: u64) -> u16 {
        unsafe { asm_vq_get_max_size(mmio_base) }
    }
    
    /// Set queue size.
    #[cfg(target_arch = "x86_64")]
    pub fn set_size(mmio_base: u64, size: u16) {
        unsafe { asm_vq_set_size(mmio_base, size) }
    }
    
    /// Enable the selected queue.
    #[cfg(target_arch = "x86_64")]
    pub fn enable(mmio_base: u64) {
        unsafe { asm_vq_enable(mmio_base) }
    }
    
    /// Full queue setup.
    #[cfg(target_arch = "x86_64")]
    pub fn setup(mmio_base: u64, queue_idx: u16, size: u16,
                 desc_addr: u64, driver_addr: u64, device_addr: u64) {
        unsafe { asm_vq_setup(mmio_base, queue_idx, size, desc_addr, driver_addr, device_addr) }
    }
    
    // Stubs for non-x86_64
    #[cfg(not(target_arch = "x86_64"))]
    pub fn select(_mmio_base: u64, _queue_idx: u16) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn get_max_size(_mmio_base: u64) -> u16 { 0 }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn set_size(_mmio_base: u64, _size: u16) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn enable(_mmio_base: u64) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn setup(_mmio_base: u64, _queue_idx: u16, _size: u16,
                 _desc_addr: u64, _driver_addr: u64, _device_addr: u64) {}
}

/// TX operations.
pub mod tx {
    use super::*;

    /// Submit packet for transmission. Fire-and-forget!
    /// Returns true on success, false if queue full.
    #[cfg(target_arch = "x86_64")]
    pub fn submit(vq: &mut VirtqueueState, buffer_idx: u16, len: u16) -> bool {
        unsafe { asm_vq_submit_tx(vq, buffer_idx, len) == 0 }
    }
    
    /// Poll for TX completion.
    /// Returns Some(buffer_idx) if a buffer completed, None otherwise.
    #[cfg(target_arch = "x86_64")]
    pub fn poll_complete(vq: &mut VirtqueueState) -> Option<u16> {
        let result = unsafe { asm_vq_poll_tx_complete(vq) };
        if result == 0xFFFFFFFF {
            None
        } else {
            Some(result as u16)
        }
    }
    
    /// Get number of available TX slots.
    #[cfg(target_arch = "x86_64")]
    pub fn available_slots(vq: &mut VirtqueueState) -> u16 {
        unsafe { asm_vq_tx_avail_slots(vq) }
    }
    
    // Stubs for non-x86_64
    #[cfg(not(target_arch = "x86_64"))]
    pub fn submit(_vq: &mut VirtqueueState, _buffer_idx: u16, _len: u16) -> bool { false }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn poll_complete(_vq: &mut VirtqueueState) -> Option<u16> { None }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn available_slots(_vq: &mut VirtqueueState) -> u16 { 0 }
}

/// RX operations.
pub mod rx {
    use super::*;

    /// Submit buffer to receive queue.
    /// Returns true on success, false if queue full.
    #[cfg(target_arch = "x86_64")]
    pub fn submit(vq: &mut VirtqueueState, buffer_idx: u16, capacity: u16) -> bool {
        unsafe { asm_vq_submit_rx(vq, buffer_idx, capacity) == 0 }
    }
    
    /// Poll for received packet.
    /// Returns Some(RxResult) if packet ready, None otherwise.
    #[cfg(target_arch = "x86_64")]
    pub fn poll(vq: &mut VirtqueueState) -> Option<RxResult> {
        let mut result = RxResult::default();
        if unsafe { asm_vq_poll_rx(vq, &mut result) } == 1 {
            Some(result)
        } else {
            None
        }
    }
    
    /// Get number of pending packets.
    #[cfg(target_arch = "x86_64")]
    pub fn pending_count(vq: &mut VirtqueueState) -> u16 {
        unsafe { asm_vq_rx_pending(vq) }
    }
    
    // Stubs for non-x86_64
    #[cfg(not(target_arch = "x86_64"))]
    pub fn submit(_vq: &mut VirtqueueState, _buffer_idx: u16, _capacity: u16) -> bool { false }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn poll(_vq: &mut VirtqueueState) -> Option<RxResult> { None }
    #[cfg(not(target_arch = "x86_64"))]
    pub fn pending_count(_vq: &mut VirtqueueState) -> u16 { 0 }
}

/// Notification operations.
pub mod notify {
    use super::*;

    /// Notify device about queue activity.
    #[cfg(target_arch = "x86_64")]
    pub fn notify(vq: &mut VirtqueueState) {
        unsafe { asm_vq_notify(vq) }
    }
    
    /// Direct notify with explicit address.
    #[cfg(target_arch = "x86_64")]
    pub fn notify_direct(notify_addr: u64, queue_idx: u16) {
        unsafe { asm_vq_notify_direct(notify_addr, queue_idx) }
    }
    
    /// Check if notification is needed.
    #[cfg(target_arch = "x86_64")]
    pub fn should_notify(vq: &mut VirtqueueState) -> bool {
        unsafe { asm_vq_should_notify(vq) == 1 }
    }
    
    // Stubs for non-x86_64
    #[cfg(not(target_arch = "x86_64"))]
    pub fn notify(_vq: &mut VirtqueueState) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn notify_direct(_notify_addr: u64, _queue_idx: u16) {}
    #[cfg(not(target_arch = "x86_64"))]
    pub fn should_notify(_vq: &mut VirtqueueState) -> bool { false }
}
