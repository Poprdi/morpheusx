//! VirtIO-net driver. Transport lives in `morpheus-virtio::transport`.

pub mod config;
pub mod driver;
pub mod init;
pub mod rx;
pub mod tx;

pub use config::{features, is_virtio_net, negotiate_features, status, VirtioConfig};
pub use config::{VIRTIO_NET_DEVICE_IDS, VIRTIO_VENDOR_ID};
pub use driver::VirtioNetDriver;
pub use init::{virtio_net_init, virtio_net_init_transport, VirtioInitError};

/// VirtIO net header (12 bytes, modern). Local mirror of
/// `morpheus_net_stack::types::VirtioNetHdr` to break a dependency cycle.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioNetHdr {
    pub flags: u8,
    /// 0 = no GSO.
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
    /// Only with MRG_RXBUF.
    pub num_buffers: u16,
}

impl VirtioNetHdr {
    pub const SIZE: usize = 12;

    /// Zeroed header (correct for all our transmits).
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

    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, Self::SIZE) }
    }
}

impl Default for VirtioNetHdr {
    fn default() -> Self {
        Self::zeroed()
    }
}
