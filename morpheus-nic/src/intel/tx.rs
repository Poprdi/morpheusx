//! Intel e1000e TX path. Hardware access goes through ASM bindings.

use crate::asm::{
    asm_intel_tx_clear_desc, asm_intel_tx_init_desc, asm_intel_tx_poll, asm_intel_tx_submit,
    asm_intel_tx_update_tail,
};
use crate::serial::{serial_print, serial_print_hex, serial_println};
use morpheus_hal_x86_64::asm::barriers::sfence;

/// Size of a single TX descriptor in bytes.
pub const TX_DESC_SIZE: usize = 16;

/// Maximum Ethernet frame size (without FCS - hardware adds it).
pub const MAX_FRAME_SIZE: usize = 1514;

pub const DEFAULT_BUFFER_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    QueueFull,
    FrameTooLarge { provided: usize, max: usize },
}

/// Fire-and-forget TX ring: `transmit()` returns after submit; completions are
/// reaped separately via `collect_completions()`.
pub struct TxRing {
    mmio_base: u64,
    desc_cpu: *mut u8,
    desc_bus: u64,
    buffer_cpu: *mut u8,
    buffer_bus: u64,
    buffer_size: usize,
    queue_size: u16,
    next_to_use: u16,
    next_to_clean: u16,
}

impl TxRing {
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

    pub fn init_descriptors(&mut self) {
        serial_print("  [TX-INIT] desc_bus=0x");
        serial_print_hex(self.desc_bus);
        serial_print(" buffer_bus=0x");
        serial_print_hex(self.buffer_bus);
        serial_println("");

        // Real Intel NICs need addresses below 4GB.
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

        // SFENCE so descriptors are visible to the NIC before enable.
        sfence();
        serial_println("  [TX-INIT] Descriptors initialized + sfence");
    }

    pub fn desc_len_bytes(&self) -> u32 {
        (self.queue_size as u32) * (TX_DESC_SIZE as u32)
    }

    #[inline]
    pub fn can_transmit(&self) -> bool {
        self.available() > 0
    }

    #[inline]
    pub fn available(&self) -> u16 {
        self.queue_size - self.in_flight() - 1
    }

    /// Descriptors submitted but not yet completed.
    #[inline]
    pub fn in_flight(&self) -> u16 {
        if self.next_to_use >= self.next_to_clean {
            self.next_to_use - self.next_to_clean
        } else {
            self.queue_size - self.next_to_clean + self.next_to_use
        }
    }

    /// Fire-and-forget; does not wait for completion.
    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        if frame.len() > MAX_FRAME_SIZE {
            return Err(TxError::FrameTooLarge {
                provided: frame.len(),
                max: MAX_FRAME_SIZE,
            });
        }

        if !self.can_transmit() {
            return Err(TxError::QueueFull);
        }

        let desc_idx = self.next_to_use;
        let desc_ptr = self.desc_ptr(desc_idx);
        let buffer_cpu = self.buffer_cpu_ptr(desc_idx);
        let buffer_bus = self.buffer_bus_addr(desc_idx);

        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer_cpu, frame.len());
        }

        // Sets EOP|IFCS|RS, includes sfence.
        unsafe {
            asm_intel_tx_submit(desc_ptr, buffer_bus, frame.len() as u32);
        }

        self.next_to_use = (self.next_to_use + 1) % self.queue_size;

        unsafe {
            asm_intel_tx_update_tail(self.mmio_base, self.next_to_use as u32);
        }

        Ok(())
    }

    /// Reap completed descriptors. Call periodically (mainloop Phase 5).
    pub fn collect_completions(&mut self) {
        while self.next_to_clean != self.next_to_use {
            let desc_ptr = self.desc_ptr(self.next_to_clean);

            // Poll includes lfence.
            let is_done = unsafe { asm_intel_tx_poll(desc_ptr) };

            if is_done == 0 {
                break;
            }

            unsafe {
                asm_intel_tx_clear_desc(desc_ptr);
            }

            self.next_to_clean = (self.next_to_clean + 1) % self.queue_size;
        }
    }

    #[inline]
    fn desc_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.desc_cpu.add((idx as usize) * TX_DESC_SIZE) }
    }

    #[inline]
    fn buffer_bus_addr(&self, idx: u16) -> u64 {
        self.buffer_bus + (idx as u64) * (self.buffer_size as u64)
    }

    #[inline]
    fn buffer_cpu_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.buffer_cpu.add((idx as usize) * self.buffer_size) }
    }
}

// SAFETY: holds only raw pointers valid for the driver's lifetime.
unsafe impl Send for TxRing {}
