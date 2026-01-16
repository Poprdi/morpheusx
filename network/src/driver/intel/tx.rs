//! Intel e1000e TX path.
//!
//! Rust orchestration layer for transmit operations.
//! All hardware access is via ASM bindings.

use crate::asm::core::barriers::sfence;
use crate::asm::drivers::intel::{
    asm_intel_tx_clear_desc, asm_intel_tx_init_desc, asm_intel_tx_poll, asm_intel_tx_submit,
    asm_intel_tx_update_tail,
};
use crate::mainloop::serial::{serial_print, serial_print_hex, serial_println};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Size of a single TX descriptor in bytes.
pub const TX_DESC_SIZE: usize = 16;

/// Maximum Ethernet frame size (without FCS - hardware adds it).
pub const MAX_FRAME_SIZE: usize = 1514;

/// Default TX buffer size (2KB).
pub const DEFAULT_BUFFER_SIZE: usize = 2048;

// ═══════════════════════════════════════════════════════════════════════════
// TX ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// TX errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    /// TX queue is full.
    QueueFull,
    /// Frame too large.
    FrameTooLarge {
        /// Provided frame size.
        provided: usize,
        /// Maximum allowed size.
        max: usize,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// TX RING
// ═══════════════════════════════════════════════════════════════════════════

/// TX descriptor ring.
///
/// Manages transmit descriptor ring and associated buffers.
/// All hardware access is via ASM functions.
///
/// # Fire-and-Forget Semantics
/// `transmit()` returns immediately after submitting. Completion is
/// collected separately via `collect_completions()`.
pub struct TxRing {
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
    /// Next descriptor to use for transmit.
    next_to_use: u16,
    /// Next descriptor to check for completion.
    next_to_clean: u16,
}

impl TxRing {
    /// Create a new TX ring.
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
            next_to_use: 0,
            next_to_clean: 0,
        }
    }

    /// Initialize all descriptors to zero.
    pub fn init_descriptors(&mut self) {
        // Print critical DMA info for hardware debugging
        serial_print("  [TX-INIT] desc_bus=0x");
        serial_print_hex(self.desc_bus);
        serial_print(" buffer_bus=0x");
        serial_print_hex(self.buffer_bus);
        serial_println("");
        
        // Check if addresses are in valid range for real hardware
        if self.desc_bus > 0xFFFF_FFFF {
            serial_println("  [WARNING] TX desc_bus > 4GB!");
        }
        if self.buffer_bus > 0xFFFF_FFFF {
            serial_println("  [WARNING] TX buffer_bus > 4GB!");
        }
        
        for i in 0..self.queue_size {
            let desc_ptr = self.desc_ptr(i);
            unsafe {
                asm_intel_tx_init_desc(desc_ptr);
            }
        }
        
        // CRITICAL: SFENCE after writing all descriptors
        unsafe { sfence(); }
        serial_println("  [TX-INIT] Descriptors initialized + sfence");
    }

    /// Get descriptor ring length in bytes.
    pub fn desc_len_bytes(&self) -> u32 {
        (self.queue_size as u32) * (TX_DESC_SIZE as u32)
    }

    /// Check if we can transmit a frame.
    #[inline]
    pub fn can_transmit(&self) -> bool {
        self.available() > 0
    }

    /// Get number of available descriptors.
    #[inline]
    pub fn available(&self) -> u16 {
        self.queue_size - self.in_flight() - 1
    }

    /// Get number of descriptors in flight (submitted but not completed).
    #[inline]
    pub fn in_flight(&self) -> u16 {
        if self.next_to_use >= self.next_to_clean {
            self.next_to_use - self.next_to_clean
        } else {
            self.queue_size - self.next_to_clean + self.next_to_use
        }
    }

    /// Transmit a frame (fire-and-forget).
    ///
    /// # Arguments
    /// - `frame`: Ethernet frame to transmit
    ///
    /// # Returns
    /// - `Ok(())`: Frame queued for transmission
    /// - `Err(TxError::QueueFull)`: No descriptors available
    /// - `Err(TxError::FrameTooLarge)`: Frame exceeds maximum size
    ///
    /// # Contract
    /// Returns immediately. Does NOT wait for completion.
    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        // Check frame size
        if frame.len() > MAX_FRAME_SIZE {
            return Err(TxError::FrameTooLarge {
                provided: frame.len(),
                max: MAX_FRAME_SIZE,
            });
        }

        // Check if we have a descriptor available
        if !self.can_transmit() {
            return Err(TxError::QueueFull);
        }

        let desc_idx = self.next_to_use;
        let desc_ptr = self.desc_ptr(desc_idx);
        let buffer_cpu = self.buffer_cpu_ptr(desc_idx);
        let buffer_bus = self.buffer_bus_addr(desc_idx);

        // Copy frame to buffer
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer_cpu, frame.len());
        }

        // Submit descriptor (sets EOP, IFCS, RS, includes sfence)
        unsafe {
            asm_intel_tx_submit(desc_ptr, buffer_bus, frame.len() as u32);
        }

        // Advance next_to_use
        self.next_to_use = (self.next_to_use + 1) % self.queue_size;

        // Update tail register
        unsafe {
            asm_intel_tx_update_tail(self.mmio_base, self.next_to_use as u32);
        }

        Ok(())
    }

    /// Collect completed transmissions.
    ///
    /// Call periodically (e.g., in main loop Phase 5) to reclaim descriptors.
    pub fn collect_completions(&mut self) {
        while self.next_to_clean != self.next_to_use {
            let desc_ptr = self.desc_ptr(self.next_to_clean);

            // Check if this descriptor is done (includes lfence)
            let is_done = unsafe { asm_intel_tx_poll(desc_ptr) };

            if is_done == 0 {
                // Not done yet - stop here
                break;
            }

            // Clear descriptor for reuse
            unsafe {
                asm_intel_tx_clear_desc(desc_ptr);
            }

            // Advance next_to_clean
            self.next_to_clean = (self.next_to_clean + 1) % self.queue_size;
        }
    }

    /// Get CPU pointer to descriptor.
    #[inline]
    fn desc_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.desc_cpu.add((idx as usize) * TX_DESC_SIZE) }
    }

    /// Get bus address of buffer.
    #[inline]
    fn buffer_bus_addr(&self, idx: u16) -> u64 {
        self.buffer_bus + (idx as u64) * (self.buffer_size as u64)
    }

    /// Get CPU pointer to buffer.
    #[inline]
    fn buffer_cpu_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.buffer_cpu.add((idx as usize) * self.buffer_size) }
    }
}

// Safety: TxRing is Send as it only holds raw pointers that are valid
// for the lifetime of the driver.
unsafe impl Send for TxRing {}
