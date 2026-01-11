//! VirtIO network header definitions.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.6, VirtIO Spec §5.1.6

/// VirtIO network header (12 bytes for modern devices).
///
/// This header is prepended to every packet sent/received via VirtIO-net.
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
    /// Header size in bytes.
    pub const SIZE: usize = 12;

    /// Create a zeroed header (correct for all our transmits).
    pub const fn zeroed() -> Self {
        Self {
            flags: 0,
            gso_type: VIRTIO_NET_HDR_GSO_NONE,
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

// GSO types
/// No GSO.
pub const VIRTIO_NET_HDR_GSO_NONE: u8 = 0;
/// TCP GSO for IPv4.
pub const VIRTIO_NET_HDR_GSO_TCPV4: u8 = 1;
/// UDP GSO.
pub const VIRTIO_NET_HDR_GSO_UDP: u8 = 3;
/// TCP GSO for IPv6.
pub const VIRTIO_NET_HDR_GSO_TCPV6: u8 = 4;
/// ECN flag.
pub const VIRTIO_NET_HDR_GSO_ECN: u8 = 0x80;

// Header flags
/// Checksum is valid/needed.
pub const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;
/// Data is valid (for hash reports).
pub const VIRTIO_NET_HDR_F_DATA_VALID: u8 = 2;
