//! TRB structs and ring management.

use crate::usb::regs::*;

/// xHCI Transfer Request Block — 16 bytes, little-endian.
#[repr(C, packed)]
pub struct Trb {
    pub param_lo: u32,
    pub param_hi: u32,
    pub status: u32,
    pub ctrl: u32,
}

impl Trb {
    #[inline(always)]
    pub fn new(param: u64, status: u32, ctrl: u32) -> Self {
        Self {
            param_lo: param as u32,
            param_hi: (param >> 32) as u32,
            status,
            ctrl,
        }
    }
}

/// Write a TRB at `base + idx*16`. Ctrl written last to set cycle state atomically.
#[inline(always)]
pub unsafe fn write_trb(base: u64, idx: usize, param: u64, status: u32, ctrl: u32) {
    let a = base + (idx as u64) * 16;
    core::ptr::write_volatile((a) as *mut u32, param as u32);
    core::ptr::write_volatile((a + 4) as *mut u32, (param >> 32) as u32);
    core::ptr::write_volatile((a + 8) as *mut u32, status);
    core::ptr::write_volatile((a + 12) as *mut u32, ctrl);
}

/// Volatile read of a 32-bit value from DMA RAM.
#[inline(always)]
pub unsafe fn vr32(a: u64) -> u32 {
    core::ptr::read_volatile(a as *const u32)
}

/// Volatile write of a 32-bit value to DMA RAM.
#[inline(always)]
pub unsafe fn vw32(a: u64, v: u32) {
    core::ptr::write_volatile(a as *mut u32, v);
}

/// Volatile write of a 64-bit value to DMA RAM (two dwords).
#[inline(always)]
pub unsafe fn vw64(a: u64, v: u64) {
    vw32(a, v as u32);
    vw32(a + 4, (v >> 32) as u32);
}

/// Command ring producer.
pub struct CmdRing {
    pub base: u64,
    pub enq: u8,
    pub cycle: u8,
    pub len: u8,
}

impl CmdRing {
    pub fn new(base: u64) -> Self {
        Self { base, enq: 0, cycle: 1, len: CMD_RING_LEN }
    }

    /// Enqueue a TRB and advance producer. Wraps if ring is full.
    #[inline(always)]
    pub unsafe fn enqueue(&mut self, param: u64, status: u32, ctrl: u32) {
        let c = (ctrl & !1) | (self.cycle as u32);
        write_trb(self.base, self.enq as usize, param, status, c);
        self.enq += 1;
        if self.enq >= self.len - 1 {
            let link_ctrl = TRB_LINK | TRB_TC | (self.cycle as u32);
            write_trb(self.base, self.enq as usize, self.base, 0, link_ctrl);
            self.enq = 0;
            self.cycle ^= 1;
        }
    }
}

/// Transfer ring for an endpoint.
pub struct XferRing {
    pub base: u64,
    pub enq: u8,
    pub cycle: u8,
    pub len: u8,
}

impl XferRing {
    pub fn new(base: u64, len: u8) -> Self {
        Self { base, enq: 0, cycle: 1, len }
    }

    #[inline(always)]
    pub unsafe fn enqueue(&mut self, param: u64, status: u32, ctrl: u32) {
        let c = (ctrl & !1) | (self.cycle as u32);
        write_trb(self.base, self.enq as usize, param, status, c);
        self.enq += 1;
        if self.enq >= self.len - 1 {
            let link = TRB_LINK | TRB_TC | (self.cycle as u32);
            write_trb(self.base, self.enq as usize, self.base, 0, link);
            self.enq = 0;
            self.cycle ^= 1;
        }
    }

    pub fn reset(&mut self) {
        self.enq = 0;
        self.cycle = 1;
    }
}

/// Event ring consumer.
pub struct EvtRing {
    pub base: u64,
    pub deq: u8,
    pub cycle: u8,
    pub len: u8,
}

impl EvtRing {
    pub fn new(base: u64) -> Self {
        Self { base, deq: 0, cycle: 1, len: EVT_RING_LEN }
    }

    /// Read the current event TRB without advancing (for inspection).
    #[inline(always)]
    pub unsafe fn peek(&self) -> Option<(u64, u32, u32)> {
        let a = self.base + (self.deq as u64) * 16;
        let ctrl = vr32(a + 12);
        if (ctrl & 1) != self.cycle as u32 {
            return None;
        }
        let param_lo = vr32(a) as u64;
        let param_hi = vr32(a + 4) as u64;
        let status = vr32(a + 8);
        Some((param_lo | (param_hi << 32), status, ctrl))
    }

    /// Advance past the current event.
    #[inline(always)]
    pub fn advance(&mut self) {
        self.deq += 1;
        if self.deq >= self.len {
            self.deq = 0;
            self.cycle ^= 1;
        }
    }
}