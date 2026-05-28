//! Intel e1000e RX path. Hardware access goes through ASM bindings.

use crate::asm::{
    asm_intel_rx_clear_desc, asm_intel_rx_init_desc, asm_intel_rx_poll, asm_intel_rx_update_tail,
    RxPollResult,
};
use crate::serial::{serial_print, serial_print_hex, serial_println};
use morpheus_hal_x86_64::asm::barriers::{lfence, sfence};

/// Size of a single RX descriptor in bytes.
pub const RX_DESC_SIZE: usize = 16;

pub const DEFAULT_BUFFER_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    BufferTooSmall { needed: usize, provided: usize },
    PacketError(u8),
}

pub struct RxRing {
    mmio_base: u64,
    desc_cpu: *mut u8,
    desc_bus: u64,
    buffer_cpu: *mut u8,
    buffer_bus: u64,
    buffer_size: usize,
    queue_size: u16,
    next_to_clean: u16,
    /// Last tail value written to hardware.
    tail: u16,
}

impl RxRing {
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

    pub fn init_descriptors(&mut self) {
        serial_print("  [RX-INIT] desc_bus=0x");
        serial_print_hex(self.desc_bus);
        serial_print(" buffer_bus=0x");
        serial_print_hex(self.buffer_bus);
        serial_println("");

        // Real Intel NICs need addresses below 4GB (absent a 64-bit BAR config).
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

        // SFENCE so descriptors are visible to the NIC before RX enable. Real
        // hardware fails without it; QEMU ignores it.
        sfence();
        serial_println("  [RX-INIT] Descriptors initialized + sfence");
    }

    /// Call after init_descriptors. Tail = last valid descriptor.
    pub fn update_tail(&mut self) {
        self.tail = self.queue_size - 1;
        unsafe {
            asm_intel_rx_update_tail(self.mmio_base, self.tail as u32);
        }
    }

    pub fn desc_len_bytes(&self) -> u32 {
        (self.queue_size as u32) * (RX_DESC_SIZE as u32)
    }

    #[inline]
    pub fn can_receive(&self) -> bool {
        let desc_ptr = self.desc_ptr(self.next_to_clean);
        let mut result = RxPollResult::default();

        unsafe { asm_intel_rx_poll(desc_ptr, &mut result) != 0 }
    }

    pub fn receive(&mut self, out_buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        let desc_idx = self.next_to_clean;
        let desc_ptr = self.desc_ptr(desc_idx);
        let mut result = RxPollResult::default();

        // Poll includes lfence.
        let has_packet = unsafe { asm_intel_rx_poll(desc_ptr, &mut result) };

        if has_packet == 0 {
            return Ok(None);
        }

        if result.has_errors() {
            self.release_descriptor(desc_idx); // release even on error
            return Err(RxError::PacketError(result.errors));
        }

        let length = result.length as usize;

        if out_buffer.len() < length {
            self.release_descriptor(desc_idx); // release even on error
            return Err(RxError::BufferTooSmall {
                needed: length,
                provided: out_buffer.len(),
            });
        }

        // DD set means the descriptor write landed, but buffer data may not be
        // visible yet. LFENCE matches Linux dma_rmb() in e1000_clean_rx_irq;
        // without it real hardware yields stale/partial data.
        lfence();

        let buffer_ptr = self.buffer_cpu_ptr(desc_idx);
        unsafe {
            core::ptr::copy_nonoverlapping(buffer_ptr, out_buffer.as_mut_ptr(), length);
        }

        self.release_descriptor(desc_idx);

        Ok(Some(length))
    }

    /// Clear status (keep buffer addr) and hand the descriptor back to hardware.
    fn release_descriptor(&mut self, idx: u16) {
        let desc_ptr = self.desc_ptr(idx);

        unsafe {
            asm_intel_rx_clear_desc(desc_ptr);
        }

        self.next_to_clean = (self.next_to_clean + 1) % self.queue_size;

        // Tail points to the descriptor BEFORE the first one hardware may use.
        let old_tail = self.tail;
        self.tail = idx;

        if self.tail != old_tail {
            unsafe {
                asm_intel_rx_update_tail(self.mmio_base, self.tail as u32);
            }
        }
    }

    #[inline]
    fn desc_ptr(&self, idx: u16) -> *mut u8 {
        unsafe { self.desc_cpu.add((idx as usize) * RX_DESC_SIZE) }
    }

    #[inline]
    fn buffer_bus_addr(&self, idx: u16) -> u64 {
        self.buffer_bus + (idx as u64) * (self.buffer_size as u64)
    }

    #[inline]
    fn buffer_cpu_ptr(&self, idx: u16) -> *const u8 {
        unsafe { self.buffer_cpu.add((idx as usize) * self.buffer_size) }
    }
}

// SAFETY: holds only raw pointers valid for the driver's lifetime.
unsafe impl Send for RxRing {}
