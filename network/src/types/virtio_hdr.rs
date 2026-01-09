//! VirtIO network header definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, VirtIO Spec §5.1.6

// TODO: Implement VirtioNetHdr
//
// /// VirtIO network header (12 bytes for modern devices).
// #[repr(C)]
// pub struct VirtioNetHdr {
//     pub flags: u8,
//     pub gso_type: u8,
//     pub hdr_len: u16,
//     pub gso_size: u16,
//     pub csum_start: u16,
//     pub csum_offset: u16,
//     pub num_buffers: u16,
// }
//
// impl VirtioNetHdr {
//     pub const SIZE: usize = 12;
//     
//     pub const fn zeroed() -> Self { ... }
// }
//
// // GSO types
// pub const VIRTIO_NET_HDR_GSO_NONE: u8 = 0;
