//! Ethernet II frame types. IEEE 802.3.

pub const ETH_ALEN: usize = 6;
pub const ETH_HLEN: usize = 14;
pub const ETH_MTU: usize = 1500;
pub const ETH_FRAME_MAX: usize = 1514;

pub type MacAddress = [u8; 6];

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct EthernetHeader {
    pub dest: MacAddress,
    pub src: MacAddress,
    /// Network byte order.
    pub ethertype: [u8; 2],
}

impl EthernetHeader {
    pub fn ethertype_u16(&self) -> u16 {
        u16::from_be_bytes(self.ethertype)
    }

    pub fn set_ethertype(&mut self, val: u16) {
        self.ethertype = val.to_be_bytes();
    }
}

// EtherTypes in host byte order.
pub const ETH_P_IP: u16 = 0x0800;
pub const ETH_P_ARP: u16 = 0x0806;
pub const ETH_P_IPV6: u16 = 0x86DD;
pub const ETH_P_8021Q: u16 = 0x8100;

pub const MAC_BROADCAST: MacAddress = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

pub fn is_broadcast(mac: &MacAddress) -> bool {
    *mac == MAC_BROADCAST
}

/// I/G bit (byte 0, bit 0).
pub fn is_multicast(mac: &MacAddress) -> bool {
    mac[0] & 0x01 != 0
}

/// U/L bit (byte 0, bit 1).
pub fn is_local(mac: &MacAddress) -> bool {
    mac[0] & 0x02 != 0
}

/// Sets U/L=1, I/G=0.
pub fn generate_local_mac(seed: u64) -> MacAddress {
    let mut mac = [0u8; 6];
    let bytes = seed.to_le_bytes();
    mac[0] = (bytes[0] & 0xFE) | 0x02;
    mac[1] = bytes[1];
    mac[2] = bytes[2];
    mac[3] = bytes[3];
    mac[4] = bytes[4];
    mac[5] = bytes[5];
    mac
}
