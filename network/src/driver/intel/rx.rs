//! Intel e1000e RX path.
//!
//! Rust orchestration layer for receive operations.
//! All hardware access is via ASM bindings.

use crate::asm::core::barriers::{lfence, sfence};
use crate::asm::drivers::intel::{
    asm_intel_rx_clear_desc, asm_intel_rx_init_desc, asm_intel_rx_poll, asm_intel_rx_update_tail,
    RxPollResult,
};
use crate::mainloop::serial::{serial_print, serial_print_hex, serial_println};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Size of a single RX descriptor in bytes.
pub const RX_DESC_SIZE: usize = 16;

/// Default RX buffer size (2KB).
pub const DEFAULT_BUFFER_SIZE: usize = 2048;

// ═══════════════════════════════════════════════════════════════════════════
// RX ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// RX errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    /// Provided buffer too small for received packet.
    BufferTooSmall {
        /// Required size.
        needed: usize,
        /// Provided size.
        provided: usize,
    },
    /// Packet had hardware errors.
    PacketError(u8),
}

// ═══════════════════════════════════════════════════════════════════════════
// RX RING
// ═══════════════════════════════════════════════════════════════════════════

/// RX descriptor ring.
///
/// Manages receive descriptor ring and associated buffers.
/// All hardware access is via ASM functions.
pub struct RxRing {
    /// MMIO base address.
    mmio_base: u64,
    /// CPU pointer to descriptor ring.
    desc_cpu: *mut u8,
    /// Bus address of descriptor ring.
    desc_bus: u64,
    /// CPU pointer to buffer region.
    buffer_cpu: *mut u8,
    /// Bus address of buffer region.
    buffer_bus: u64,
    /// Size of each buffer.
    buffer_size: usize,
    /// Number of descriptors.
    queue_size: u16,
    /// Next descriptor to check for received packet.
    next_to_clean: u16,
    /// Last tail value written to hardware.
    tail: u16,
}

impl RxRing {
    /// Create a new RX ring.
    ///
    /// # Safety
    /// All pointers and addresses must be valid.
    pub unsafe fn new(
        mmio_base: u64,
        desc_cpu: *mut u8,
        desc_bus: u64,
        buffer_cpu: *mut u8,
        buffer_bus: u64,
        buffer_size: usize,
        queue_size: u16,
    ) -> Self {
        Self {
            mmio_base,
            desc_cpu,
            desc_bus,
            buffer_cpu,
            buffer_bus,
            buffer_size,
            queue_size,
            next_to_clean: 0,
            tail: 0,
        }
    }

    /// Initialize all descriptors with buffer addresses.
    pub fn init_descriptors(&mut self) {
        // Print critical DMA info for hardware debugging
        serial_print("  [RX-INIT] desc_bus=0x");
        serial_print_hex(self.desc_bus);
        serial_print(" buffer_bus=0x");
        serial_print_hex(self.buffer_bus);
        serial_println("");
        
        // Check if addresses are in valid range for real hardware
        // Real Intel NICs require addresses below 4GB (or proper 64-bit BAR config)
        if self.desc_bus > 0xFFFF_FFFF {
            serial_println("  [WARNING] RX desc_bus > 4GB!");
        }
        if self.buffer_bus > 0xFFFF_FFFF {
            serial_println("  [WARNING] RX buffer_bus > 4GB!");
        }
        
        for i in 0..self.queue_size {
            let desc_ptr = self.desc_ptr(i);
            let buffer_bus = self.buffer_bus_addr(i);

            unsafe {
                asm_intel_rx_init_desc(desc_ptr, buffer_bus);
            }
        }
        
        // CRITICAL: SFENCE after writing all descriptors
        // This ensures descriptors are visible to the NIC before we enable RX
        // Real hardware WILL fail without this; QEMU ignores it
        unsafe { sfence(); }
        serial_println("  [RX-INIT] Descriptors initialized + sfence");
    }

    /// Update tail register to start receiving.
    ///
    /// Should be called after init_descriptors.
    pub fn update_tail(&mut self) {
        // Tail points to last valid descriptor
        self.tail = self.queue_size - 1;
        unsafe {
            asm_intel_rx_update_tail(self.mmio_base, self.tail as u32);
        }
    }

    /// Get descriptor ring length in bytes.
    pub fn desc_len_bytes(&self) -> u32 {
        (self.queue_size as u32) * (RX_DESC_SIZE as u32)
    }

    /// Check if a packet is available.
    #[inline]
    pub fn can_receive(&self) -> bool {
        let desc_ptr = self.desc_ptr(self.next_to_clean);
        let mut result = RxPollResult::default();

        unsafe { asm_intel_rx_poll(desc_ptr, &mut result) != 0 }
    }

    /// Receive a packet.
    ///
    /// # Arguments
    /// - `out_buffer`: Buffer to copy received packet into
    ///
    /// # Returns
    /// - `Ok(Some(len))`: Packet received, length in bytes
    /// - `Ok(None)`: No packet available
    /// - `Err(RxError)`: Error occurred
    pub fn receive(&mut self, out_buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        let desc_idx = self.next_to_clean;
        let desc_ptr = self.desc_ptr(desc_idx);
        let mut result = RxPollResult::default();

        // Poll for packet (includes lfence)
        let has_packet = unsafe { asm_intel_rx_poll(desc_ptr, &mut result) };

        if has_packet == 0 {
            return Ok(None);
        }

        // Check for errors
        if result.has_errors() {
            // Still need to release the descriptor
            self.release_descriptor(desc_idx);
            return Err(RxError::PacketError(result.errors));
        }

        let length = result.length as usize;

        // Check buffer size
        if out_buffer.len() < length {
            // Still need to release the descriptor
            self.release_descriptor(desc_idx);
            return Err(RxError::BufferTooSmall {
                needed: length,
                provided: out_buffer.len(),
            });
        }

        // CRITICAL: Memory barrier before reading buffer data
        // The device has finished writing to the descriptor (DD bit set),
        // but we need LFENCE to ensure buffer data writes are also visible.
        // This matches Linux kernel's dma_rmb() placement in e1000_clean_rx_irq.
        // Without this, we may read stale/partial buffer data on real hardware.
        lfence();

        // Copy packet data from buffer
        let buffer_ptr = self.buffer_cpu_ptr(desc_idx);
        unsafe {
            core::ptr::copy_nonoverlapping(buffer_ptr, out_buffer.as_mut_ptr(), length);
        }

        // Release descriptor for reuse
        self.release_descriptor(desc_idx);

        Ok(Some(length))
    }

    /// Release a descriptor for reuse.
    fn release_descriptor(&mut self, idx: u16) {
        let desc_ptr = self.desc_ptr(idx);

        // Clear status, keep buffer address
        unsafe {
            asm_intel_rx_clear_desc(desc_ptr);
        }

        // Advance next_to_clean
        self.next_to_clean = (self.next_to_clean + 1) % self.queue_size;

        // Update tail to release this descriptor back to hardware
        // Tail points to the descriptor BEFORE the first one hardware can use
        let old_tail = self.tail;
        self.tail = idx;

        // Only write tail if it changed
        if self.tail != old_tail {
            unsafe {
                asm_intel_rx_update_tail(self.mmio_base, self.tail as u32);
            }
        }
    }

    /// Get CPU pointer to descriptor.
    #[inline]
    fn desc_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.desc_cpu.add((idx as usize) * RX_DESC_SIZE) }
    }

    /// Get bus address of buffer.
    #[inline]
    fn buffer_bus_addr(&self, idx: u16) -> u64 {
        self.buffer_bus + (idx as u64) * (self.buffer_size as u64)
    }

    /// Get CPU pointer to buffer.
    #[inline]
    fn buffer_cpu_ptr(&self, idx: u16) -> *const u8 {
        unsafe { self.buffer_cpu.add((idx as usize) * self.buffer_size) }
    }
}

// Safety: RxRing is Send as it only holds raw pointers that are valid
// for the lifetime of the driver.
unsafe impl Send for RxRing {}
