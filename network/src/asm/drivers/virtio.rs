//! VirtIO ASM bindings.
//!
//! All VirtIO-specific assembly function declarations.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.2, §4

use crate::types::repr_c::{VirtqueueState, RxResult};

// TODO: Implement extern declarations
//
// extern "win64" {
//     // Initialization
//     pub fn asm_nic_reset(mmio_base: u64) -> u32;
//     pub fn asm_nic_set_status(mmio_base: u64, status: u8);
//     pub fn asm_nic_get_status(mmio_base: u64) -> u8;
//     pub fn asm_nic_read_features(mmio_base: u64) -> u64;
//     pub fn asm_nic_write_features(mmio_base: u64, features: u64);
//     pub fn asm_nic_read_mac(mmio_base: u64, mac_out: *mut [u8; 6]) -> u32;
//
//     // Virtqueue operations
//     pub fn asm_vq_submit_tx(vq: *mut VirtqueueState, idx: u16, len: u16) -> u32;
//     pub fn asm_vq_poll_tx_complete(vq: *mut VirtqueueState) -> u32;
//     pub fn asm_vq_submit_rx(vq: *mut VirtqueueState, idx: u16, cap: u16) -> u32;
//     pub fn asm_vq_poll_rx(vq: *mut VirtqueueState, result: *mut RxResult) -> u32;
//     pub fn asm_vq_notify(vq: *mut VirtqueueState);
// }
