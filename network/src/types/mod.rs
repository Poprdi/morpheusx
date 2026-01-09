//! Shared data types module.
//!
//! Contains all #[repr(C)] structures that are shared between Rust and ASM,
//! as well as other common type definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, ARCHITECTURE_V3.md

pub mod repr_c;
pub mod virtio_hdr;
pub mod ethernet;
pub mod result;

// Re-exports
pub use repr_c::{VirtqueueState, RxResult, VirtqDesc, DriverState, RxPollResult, TxPollResult};
pub use virtio_hdr::{VirtioNetHdr, VIRTIO_NET_HDR_GSO_NONE};
pub use ethernet::{MacAddress, EthernetHeader, ETH_ALEN, ETH_HLEN, ETH_MTU, ETH_FRAME_MAX};
pub use result::AsmResult;
