//! #[repr(C)] structures for ASM interop.
//!
//! CRITICAL: These structures MUST match the ASM layout exactly.
//! Any changes require corresponding ASM updates.
//!
//! Phase 3.1 Wave 1 — VirtIO queue structs (`VirtqueueState`, `RxResult`,
//! `VirtqDesc`, `VirtqAvailHeader`, `VirtqUsedElem`, `VirtqUsedHeader`)
//! moved to the `morpheus-virtio` crate. The structs that remain here
//! are non-VirtIO generic driver state types used by future or
//! non-VirtIO drivers.

/// Generic driver state (for future non-VirtIO drivers).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DriverState {
    /// MMIO base address.
    pub mmio_base: u64,
    /// Descriptor/ring base address.
    pub desc_base: u64,
    /// TX ring base (or avail ring).
    pub tx_ring_base: u64,
    /// RX ring base (or used ring).
    pub rx_ring_base: u64,
    /// Queue/ring size.
    pub queue_size: u16,
    /// Queue index.
    pub queue_index: u16,
    /// Padding.
    pub _pad: u32,
    /// Notification address.
    pub notify_addr: u64,
    /// Last seen index.
    pub last_seen_idx: u16,
    /// Next submit index.
    pub next_submit_idx: u16,
    /// Padding.
    pub _pad2: u32,
    /// Buffer region base.
    pub buffer_base: u64,
    /// Size of each buffer.
    pub buffer_size: u32,
    /// Number of buffers.
    pub buffer_count: u32,
    /// Reserved for future use.
    pub _reserved: [u64; 4],
}

impl DriverState {
    pub const fn new() -> Self {
        Self {
            mmio_base: 0,
            desc_base: 0,
            tx_ring_base: 0,
            rx_ring_base: 0,
            queue_size: 0,
            queue_index: 0,
            _pad: 0,
            notify_addr: 0,
            last_seen_idx: 0,
            next_submit_idx: 0,
            _pad2: 0,
            buffer_base: 0,
            buffer_size: 0,
            buffer_count: 0,
            _reserved: [0; 4],
        }
    }
}

impl Default for DriverState {
    fn default() -> Self {
        Self::new()
    }
}

/// RX poll result (generic for all drivers).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RxPollResult {
    /// Buffer index.
    pub buffer_idx: u16,
    /// Received length.
    pub length: u16,
    /// Driver-specific flags.
    pub flags: u32,
}

/// TX poll result (generic for all drivers).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TxPollResult {
    /// Buffer index.
    pub buffer_idx: u16,
    /// Status code.
    pub status: u16,
    /// Reserved.
    pub _reserved: u32,
}
