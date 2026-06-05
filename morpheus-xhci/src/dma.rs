//! DMA region for xHCI transfer rings and descriptor buffers.
//! Identity-mapped; all offsets are 64-byte aligned within a 64KB-aligned page.

pub const CMD_RING_LEN: u8 = 32;
pub const EVT_RING_LEN: u8 = 32;
pub const XFER_RING_LEN: u8 = 16;

pub const OFF_DCBAA: usize = 0x0000; // 2KB
pub const OFF_CMD_RING: usize = 0x1000; // 512B
pub const OFF_EVT_RING: usize = 0x1200; // 512B
pub const OFF_ERST: usize = 0x1400; // 16B
pub const OFF_OUT_CTX: usize = 0x2000; // (DEPRECATED — see OUT_CTX_ARRAY)
pub const OFF_IN_CTX: usize = 0x3000; // 2.5KB
pub const OFF_XFER_EP0: usize = 0x4000; // 256B
pub const OFF_XFER_BOUT: usize = 0x4100; // 256B
pub const OFF_XFER_BIN: usize = 0x4200; // 256B
pub const OFF_DESC: usize = 0x4480; // descriptor scratch, spans up to OFF_DATA
pub const OFF_DATA: usize = 0x5000; // 4KB bounce buffer
pub const OFF_REPORT: usize = 0x4300; // 64B — HID interrupt report buffer (keyboard)
pub const OFF_REPORT_MOUSE: usize = 0x4340; // 64B — second HID report buffer (mouse)
pub const OFF_XFER_MOUSE: usize = 0x6000; // 256B
pub const OFF_CBW: usize = 0x4400; // 64B
pub const OFF_CSW: usize = 0x4440; // 64B
pub const OFF_SCRATCH_ARR: usize = 0x7000; // 64B
pub const OFF_SCRATCH_PG: usize = 0x8000; // scratchpad pages (up to MAX_SCRATCH × 4 KB)
pub const MAX_SCRATCH: usize = 64;
pub const DATA_BUF_SIZE: usize = 4096;

/// Usable span of OFF_DESC before OFF_DATA. Bounds descriptor reads
/// (config + report) so they never run into the bounce buffer.
pub const DESC_BUF_SIZE: usize = OFF_DATA - OFF_DESC; // 2944 B

/// Sized 4 KB per slot (worst case is 33 endpoint contexts × 64 B = 2112 B,
/// rounded up). MaxSlotsEnabled in OP_CONFIG is capped to this count so the
/// xHC can never hand out a slot ID we have no room for.
pub const OUT_CTX_STRIDE: usize = 0x1000;
pub const MAX_OUT_CTX_SLOTS: usize = 16;
pub const OFF_OUT_CTX_ARRAY: usize = 0x48000;

pub const DMA_SIZE: usize = OFF_OUT_CTX_ARRAY + MAX_OUT_CTX_SLOTS * OUT_CTX_STRIDE;

#[inline]
pub const fn slot_out_ctx_offset(slot_id: u8) -> usize {
    OFF_OUT_CTX_ARRAY + ((slot_id as usize).saturating_sub(1) * OUT_CTX_STRIDE)
}

#[repr(C, align(4096))]
pub struct XhciDma([u8; DMA_SIZE]);

pub static mut XHCI_DMA: XhciDma = XhciDma([0u8; DMA_SIZE]);

/// Returns the physical base of the static DMA region.
///
/// # Safety
/// Caller must observe the single-threaded, identity-mapped contract of the
/// static `XHCI_DMA` region (no aliasing mutable access).
#[inline(always)]
pub unsafe fn dma_base() -> u64 {
    core::ptr::addr_of!(XHCI_DMA) as u64
}

/// Zeros the entire DMA region.
///
/// # Safety
/// `base` must be the valid base of the `DMA_SIZE`-byte DMA region and the
/// caller must hold exclusive access to it.
#[inline(always)]
pub unsafe fn dma_zero(base: u64) {
    core::ptr::write_bytes(base as *mut u8, 0, DMA_SIZE);
}

/// Zeros a subrange starting at `base + off`.
///
/// # Safety
/// `base + off .. base + off + len` must lie within a valid, exclusively-owned
/// DMA mapping.
#[inline(always)]
pub unsafe fn dma_zero_range(base: u64, off: usize, len: usize) {
    core::ptr::write_bytes((base + off as u64) as *mut u8, 0, len);
}
