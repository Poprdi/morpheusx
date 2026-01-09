//! DMA region definition and layout.
//!
//! # Memory Layout (2MB Region)
//! ```text
//! Offset      Size        Content
//! 0x00000     0x0200      RX Descriptor Table (32 × 16 bytes)
//! 0x00200     0x0048      RX Available Ring
//! 0x00400     0x0108      RX Used Ring
//! 0x00800     0x0200      TX Descriptor Table (32 × 16 bytes)
//! 0x00A00     0x0048      TX Available Ring
//! 0x00C00     0x0108      TX Used Ring
//! 0x01000     0x10000     RX Buffers (32 × 2KB)
//! 0x11000     0x10000     TX Buffers (32 × 2KB)
//! ```
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.3

// TODO: Implement DmaRegion
//
// pub struct DmaRegion {
//     pub cpu_ptr: *mut u8,
//     pub bus_addr: u64,
//     pub size: usize,
// }
//
// impl DmaRegion {
//     pub const MIN_SIZE: usize = 2 * 1024 * 1024; // 2MB
//     
//     pub fn rx_desc_offset() -> usize { 0x0000 }
//     pub fn tx_desc_offset() -> usize { 0x0800 }
//     pub fn rx_buffers_offset() -> usize { 0x1000 }
//     pub fn tx_buffers_offset() -> usize { 0x11000 }
// }
