//! #[repr(C)] structures for ASM interop.
//!
//! CRITICAL: These structures MUST match the ASM layout exactly.
//! Any changes require corresponding ASM updates.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6

/// Virtqueue state passed to ASM functions.
///
/// This structure is shared between Rust and ASM for virtqueue operations.
/// Layout must match ASM expectations exactly.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct VirtqueueState {
    /// Base address of descriptor table (bus address for device).
    pub desc_base: u64,
    /// Base address of available (driver) ring.
    pub avail_base: u64,
    /// Base address of used (device) ring.
    pub used_base: u64,
    /// Queue size (number of descriptors).
    pub queue_size: u16,
    /// Queue index (0=RX, 1=TX for virtio-net).
    pub queue_index: u16,
    /// Padding for alignment.
    pub _pad: u32,
    /// MMIO address for queue notification.
    pub notify_addr: u64,
    /// Last seen used index (for polling).
    pub last_used_idx: u16,
    /// Next available index (for submission).
    pub next_avail_idx: u16,
    /// Padding.
    pub _pad2: u32,
    /// CPU pointer to descriptor table (for driver access).
    pub desc_cpu_ptr: u64,
    /// CPU pointer to buffer region.
    pub buffer_cpu_base: u64,
    /// Bus address of buffer region.
    pub buffer_bus_base: u64,
    /// Size of each buffer.
    pub buffer_size: u32,
    /// Number of buffers.
    pub buffer_count: u32,
}

impl VirtqueueState {
    /// Create a new zeroed VirtqueueState.
    pub const fn new() -> Self {
        Self {
            desc_base: 0,
            avail_base: 0,
            used_base: 0,
            queue_size: 0,
            queue_index: 0,
            _pad: 0,
            notify_addr: 0,
            last_used_idx: 0,
            next_avail_idx: 0,
            _pad2: 0,
            desc_cpu_ptr: 0,
            buffer_cpu_base: 0,
            buffer_bus_base: 0,
            buffer_size: 0,
            buffer_count: 0,
        }
    }
}

impl Default for VirtqueueState {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from asm_vq_poll_rx.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RxResult {
    /// Index of buffer containing received packet.
    pub buffer_idx: u16,
    /// Length of received data (including VirtIO header).
    pub length: u16,
    /// Reserved for future use.
    pub _reserved: u32,
}

impl RxResult {
    pub const fn new() -> Self {
        Self {
            buffer_idx: 0,
            length: 0,
            _reserved: 0,
        }
    }
}

impl Default for RxResult {
    fn default() -> Self {
        Self::new()
    }
}

/// VirtIO descriptor (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqDesc {
    /// Buffer physical/bus address.
    pub addr: u64,
    /// Buffer length in bytes.
    pub len: u32,
    /// Flags (NEXT=1, WRITE=2, INDIRECT=4).
    pub flags: u16,
    /// Next descriptor index (if NEXT flag set).
    pub next: u16,
}

impl VirtqDesc {
    /// NEXT flag - descriptor continues in `next` field.
    pub const FLAG_NEXT: u16 = 1;
    /// WRITE flag - buffer is device-writable (for RX).
    pub const FLAG_WRITE: u16 = 2;
    /// INDIRECT flag - buffer contains indirect descriptor table.
    pub const FLAG_INDIRECT: u16 = 4;

    pub const fn new() -> Self {
        Self {
            addr: 0,
            len: 0,
            flags: 0,
            next: 0,
        }
    }
}

/// VirtIO available ring header.
#[repr(C)]
#[derive(Debug)]
pub struct VirtqAvailHeader {
    /// Flags (0 = no interrupt suppression).
    pub flags: u16,
    /// Index of next entry to write.
    pub idx: u16,
}

/// VirtIO used ring element.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsedElem {
    /// Index of descriptor chain head.
    pub id: u32,
    /// Total bytes written/consumed.
    pub len: u32,
}

/// VirtIO used ring header.
#[repr(C)]
#[derive(Debug)]
pub struct VirtqUsedHeader {
    /// Flags (0 = no notification suppression).
    pub flags: u16,
    /// Index of next entry device will write.
    pub idx: u16,
}

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
