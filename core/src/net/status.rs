//! Net init status.

#[derive(Debug, Clone)]
pub struct NetworkStatus {
    pub ip_address: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns_server: Option<[u8; 4]>,
    pub mac_address: [u8; 6],
    pub init_time_ms: u64,
    pub is_dhcp: bool,
}

impl NetworkStatus {
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

    /// Dotted-quad, space-padded to 15 bytes.
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
