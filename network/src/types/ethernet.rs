//! Ethernet frame types and helpers.
//!
//! # Reference
//! IEEE 802.3

// TODO: Implement Ethernet types
//
// pub const ETH_ALEN: usize = 6;
// pub const ETH_HLEN: usize = 14;
// pub const ETH_MTU: usize = 1500;
// pub const ETH_FRAME_MAX: usize = 1514;
//
// /// MAC address type.
// pub type MacAddress = [u8; 6];
//
// /// Ethernet header.
// #[repr(C, packed)]
// pub struct EthernetHeader {
//     pub dest: MacAddress,
//     pub src: MacAddress,
//     pub ethertype: u16,
// }
//
// // Common EtherTypes
// pub const ETH_P_IP: u16 = 0x0800;
// pub const ETH_P_ARP: u16 = 0x0806;
// pub const ETH_P_IPV6: u16 = 0x86DD;
