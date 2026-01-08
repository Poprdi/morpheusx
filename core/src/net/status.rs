//! Network initialization status.
//!
//! Status information returned after successful network initialization.

/// Network status after successful initialization.
#[derive(Debug, Clone)]
pub struct NetworkStatus {
    /// Assigned IPv4 address.
    pub ip_address: [u8; 4],
    /// Subnet mask.
    pub subnet_mask: [u8; 4],
    /// Gateway address.
    pub gateway: [u8; 4],
    /// DNS server (if provided by DHCP).
    pub dns_server: Option<[u8; 4]>,
    /// MAC address of the network device.
    pub mac_address: [u8; 6],
    /// Time taken for initialization in milliseconds.
    pub init_time_ms: u64,
    /// Whether IP was assigned via DHCP or static.
    pub is_dhcp: bool,
}

impl NetworkStatus {
    /// Create a new network status (placeholder for testing).
    pub fn new() -> Self {
        Self {
            ip_address: [0, 0, 0, 0],
            subnet_mask: [0, 0, 0, 0],
            gateway: [0, 0, 0, 0],
            dns_server: None,
            mac_address: [0, 0, 0, 0, 0, 0],
            init_time_ms: 0,
            is_dhcp: true,
        }
    }

    /// Format IP address as string for display.
    ///
    /// Returns a fixed-size array that can be converted to &str.
    pub fn ip_str(&self) -> [u8; 15] {
        let mut buf = [b' '; 15];
        let mut pos = 0;
        for (i, &octet) in self.ip_address.iter().enumerate() {
            if i > 0 {
                buf[pos] = b'.';
                pos += 1;
            }
            pos += write_u8(&mut buf[pos..], octet);
        }
        buf
    }

    /// Check if we have a valid (non-zero) IP address.
    pub fn has_ip(&self) -> bool {
        self.ip_address != [0, 0, 0, 0]
    }
}

impl Default for NetworkStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// Write u8 to buffer, return bytes written.
fn write_u8(buf: &mut [u8], val: u8) -> usize {
    if val >= 100 {
        buf[0] = b'0' + (val / 100);
        buf[1] = b'0' + ((val / 10) % 10);
        buf[2] = b'0' + (val % 10);
        3
    } else if val >= 10 {
        buf[0] = b'0' + (val / 10);
        buf[1] = b'0' + (val % 10);
        2
    } else {
        buf[0] = b'0' + val;
        1
    }
}
