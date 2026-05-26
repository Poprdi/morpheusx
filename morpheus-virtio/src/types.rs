//! `#[repr(C)]` structures shared between Rust and `asm/virtio/*.s`.
//!
//! CRITICAL: layout MUST match the ASM side exactly.

/// Virtqueue state passed to ASM helpers. Layout must match `asm/virtio/*.s`.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct VirtqueueState {
    pub desc_base: u64,
    pub avail_base: u64,
    pub used_base: u64,
    pub queue_size: u16,
    /// 0=RX, 1=TX for virtio-net.
    pub queue_index: u16,
    pub _pad: u32,
    pub notify_addr: u64,
    pub last_used_idx: u16,
    pub next_avail_idx: u16,
    pub _pad2: u32,
    pub desc_cpu_ptr: u64,
    pub buffer_cpu_base: u64,
    pub buffer_bus_base: u64,
    pub buffer_size: u32,
    pub buffer_count: u32,
}

impl VirtqueueState {
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

/// Result from `asm_vq_poll_rx`. `length` includes the VirtIO header.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RxResult {
    pub buffer_idx: u16,
    pub length: u16,
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

/// VirtIO descriptor (16 bytes). VirtIO 1.1 §2.6.5.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

impl VirtqDesc {
    pub const FLAG_NEXT: u16 = 1;
    pub const FLAG_WRITE: u16 = 2;
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

/// VirtIO available ring header. VirtIO 1.1 §2.6.6.
#[repr(C)]
#[derive(Debug)]
pub struct VirtqAvailHeader {
    pub flags: u16,
    pub idx: u16,
}

/// VirtIO used ring element. VirtIO 1.1 §2.6.8.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

/// VirtIO used ring header. VirtIO 1.1 §2.6.8.
#[repr(C)]
#[derive(Debug)]
pub struct VirtqUsedHeader {
    pub flags: u16,
    pub idx: u16,
}
