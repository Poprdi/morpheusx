//! `#[repr(C)]` structs for ASM interop. Layout MUST match the ASM exactly;
//! any field change requires a corresponding ASM update.

/// Generic driver state (for future non-VirtIO drivers).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct DriverState {
    pub mmio_base: u64,
    pub desc_base: u64,
    pub tx_ring_base: u64,
    pub rx_ring_base: u64,
    pub queue_size: u16,
    pub queue_index: u16,
    pub _pad: u32,
    pub notify_addr: u64,
    pub last_seen_idx: u16,
    pub next_submit_idx: u16,
    pub _pad2: u32,
    pub buffer_base: u64,
    pub buffer_size: u32,
    pub buffer_count: u32,
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

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RxPollResult {
    pub buffer_idx: u16,
    pub length: u16,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TxPollResult {
    pub buffer_idx: u16,
    pub status: u16,
    pub _reserved: u32,
}
