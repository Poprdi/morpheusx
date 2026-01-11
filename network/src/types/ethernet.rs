//! Ethernet frame types and helpers.
//!
//! # Reference
//! IEEE 802.3

/// Ethernet address length (6 bytes).
pub const ETH_ALEN: usize = 6;

/// Ethernet header length (14 bytes).
pub const ETH_HLEN: usize = 14;

/// Maximum Ethernet payload (MTU).
pub const ETH_MTU: usize = 1500;

/// Maximum Ethernet frame size (header + payload).
pub const ETH_FRAME_MAX: usize = 1514;

/// MAC address type.
pub type MacAddress = [u8; 6];

/// Ethernet header.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct EthernetHeader {
    /// Destination MAC address.
    pub dest: MacAddress,
    /// Source MAC address.
    pub src: MacAddress,
    /// EtherType (in network byte order).
    pub ethertype: [u8; 2],
}

impl EthernetHeader {
    /// Get EtherType as u16 (host byte order).
    pub fn ethertype_u16(&self) -> u16 {
        u16::from_be_bytes(self.ethertype)
    }

    /// Set EtherType from u16.
    pub fn set_ethertype(&mut self, val: u16) {
        self.ethertype = val.to_be_bytes();
    }
}

// Common EtherTypes (in host byte order)
/// IPv4.
pub const ETH_P_IP: u16 = 0x0800;
/// ARP.
pub const ETH_P_ARP: u16 = 0x0806;
/// IPv6.
pub const ETH_P_IPV6: u16 = 0x86DD;
/// VLAN tag.
pub const ETH_P_8021Q: u16 = 0x8100;

/// Broadcast MAC address.
pub const MAC_BROADCAST: MacAddress = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// Check if MAC is broadcast.
pub fn is_broadcast(mac: &MacAddress) -> bool {
    *mac == MAC_BROADCAST
}

/// Check if MAC is multicast (bit 0 of first byte set).
pub fn is_multicast(mac: &MacAddress) -> bool {
    mac[0] & 0x01 != 0
}

/// Check if MAC is locally administered (bit 1 of first byte set).
pub fn is_local(mac: &MacAddress) -> bool {
    mac[0] & 0x02 != 0
}

/// Generate a locally-administered MAC address from a seed.
pub fn generate_local_mac(seed: u64) -> MacAddress {
    let mut mac = [0u8; 6];
    let bytes = seed.to_le_bytes();
    mac[0] = (bytes[0] & 0xFE) | 0x02; // Clear multicast, set local
    mac[1] = bytes[1];
    mac[2] = bytes[2];
    mac[3] = bytes[3];
    mac[4] = bytes[4];
    mac[5] = bytes[5];
    mac
}
