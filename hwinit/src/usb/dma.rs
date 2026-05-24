//! DMA region for xHCI transfer rings and descriptor buffers.
//! Identity-mapped; all offsets are 64-byte aligned within a 64KB-aligned page.

pub const DMA_SIZE: usize = 0x48000;
pub const CMD_RING_LEN: u8 = 32;
pub const EVT_RING_LEN: u8 = 32;
pub const XFER_RING_LEN: u8 = 16;

pub const OFF_DCBAA: usize = 0x0000; // 2KB
pub const OFF_CMD_RING: usize = 0x1000; // 512B
pub const OFF_EVT_RING: usize = 0x1200; // 512B
pub const OFF_ERST: usize = 0x1400; // 16B
pub const OFF_OUT_CTX: usize = 0x2000; // 2KB
pub const OFF_IN_CTX: usize = 0x3000; // 2.5KB
pub const OFF_XFER_EP0: usize = 0x4000; // 256B
pub const OFF_XFER_BOUT: usize = 0x4100; // 256B
pub const OFF_XFER_BIN: usize = 0x4200; // 256B
pub const OFF_DESC: usize = 0x4480; // 256B
pub const OFF_DATA: usize = 0x5000; // 4KB bounce buffer
pub const OFF_REPORT: usize = 0x4300; // 64B — HID interrupt report buffer
pub const OFF_CBW: usize = 0x4400; // 64B
pub const OFF_CSW: usize = 0x4440; // 64B
pub const OFF_SCRATCH_ARR: usize = 0x7000; // 64B
pub const OFF_SCRATCH_PG: usize = 0x8000; // scratchpad pages
pub const MAX_SCRATCH: usize = 64;
pub const DATA_BUF_SIZE: usize = 4096;

#[repr(C, align(4096))]
pub struct XhciDma([u8; DMA_SIZE]);

pub static mut XHCI_DMA: XhciDma = XhciDma([0u8; DMA_SIZE]);

/// Returns the physical base of the static DMA region.
#[inline(always)]
pub unsafe fn dma_base() -> u64 {
    core::ptr::addr_of!(XHCI_DMA) as u64
}

/// Zeros the entire DMA region.
#[inline(always)]
pub unsafe fn dma_zero(base: u64) {
    core::ptr::write_bytes(base as *mut u8, 0, DMA_SIZE);
}

/// Zeros a subrange starting at `base + off`.
#[inline(always)]
pub unsafe fn dma_zero_range(base: u64, off: usize, len: usize) {
    core::ptr::write_bytes((base + off as u64) as *mut u8, 0, len);
}
