//! VirtIO-net driver.
//!
//! Phase 3.1 Wave 1 — `transport` module moved to `morpheus-virtio::transport`.
//! Phase 3.1 Wave 3 — `config`, `driver`, `init`, `rx`, `tx` now live here.

pub mod config;
pub mod driver;
pub mod init;
pub mod rx;
pub mod tx;

pub use config::{features, is_virtio_net, negotiate_features, status, VirtioConfig};
pub use config::{VIRTIO_NET_DEVICE_IDS, VIRTIO_VENDOR_ID};
pub use driver::VirtioNetDriver;
pub use init::{virtio_net_init, virtio_net_init_transport, VirtioInitError};

/// VirtIO network header (12 bytes for modern devices).
///
/// Mirror of `morpheus_net_stack::types::VirtioNetHdr`. Defined locally to
/// avoid a `morpheus-nic -> morpheus-net-stack -> morpheus-nic` dependency
/// cycle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioNetHdr {
    /// Header flags.
    pub flags: u8,
    /// GSO type (0 = none).
    pub gso_type: u8,
    /// Ethernet + IP + TCP/UDP header length hint.
    pub hdr_len: u16,
    /// GSO segment size.
    pub gso_size: u16,
    /// Checksum start offset.
    pub csum_start: u16,
    /// Checksum offset from csum_start.
    pub csum_offset: u16,
    /// Number of buffers (only with MRG_RXBUF).
    pub num_buffers: u16,
}

impl VirtioNetHdr {
    pub const SIZE: usize = 12;

    /// Create a zeroed header (correct for all our transmits).
    pub const fn zeroed() -> Self {
        Self {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
            num_buffers: 0,
        }
    }

    /// Get header as byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, Self::SIZE) }
    }
}

impl Default for VirtioNetHdr {
    fn default() -> Self {
        Self::zeroed()
    }
}
