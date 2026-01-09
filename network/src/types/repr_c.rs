//! #[repr(C)] structures for ASM interop.
//!
//! CRITICAL: These structures MUST match the ASM layout exactly.
//! Any changes require corresponding ASM updates.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6

// TODO: Implement the following structures:
//
// /// Virtqueue state passed to ASM functions.
// #[repr(C)]
// pub struct VirtqueueState {
//     pub desc_base: u64,
//     pub avail_base: u64,
//     pub used_base: u64,
//     pub queue_size: u16,
//     pub queue_index: u16,
//     pub _pad: u32,
//     pub notify_addr: u64,
//     pub last_used_idx: u16,
//     pub next_avail_idx: u16,
//     pub _pad2: u32,
//     pub desc_cpu_ptr: u64,
//     pub buffer_cpu_base: u64,
//     pub buffer_bus_base: u64,
//     pub buffer_size: u32,
//     pub buffer_count: u32,
// }
//
// /// Result from asm_vq_poll_rx.
// #[repr(C)]
// pub struct RxResult {
//     pub buffer_idx: u16,
//     pub length: u16,
//     pub _reserved: u32,
// }
//
// /// Generic driver state (for future drivers).
// #[repr(C)]
// pub struct DriverState {
//     pub mmio_base: u64,
//     pub desc_base: u64,
//     pub avail_base: u64,
//     pub used_base: u64,
//     pub queue_size: u16,
//     pub queue_index: u16,
//     pub notify_addr: u64,
//     pub last_used_idx: u16,
//     pub next_avail_idx: u16,
//     pub buffer_base: u64,
//     pub buffer_size: u32,
//     pub buffer_count: u32,
//     pub _reserved: [u64; 4],
// }
