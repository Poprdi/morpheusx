//! ICMP packet build/parse (RFC 792).

use crate::checksum::calculate_checksum;
use crate::types::Ipv4Addr;

pub const ICMP_PROTOCOL: u8 = 1;

pub const IP_HEADER_SIZE: usize = 20;

pub const ICMP_HEADER_SIZE: usize = 8;

pub const MIN_PACKET_SIZE: usize = IP_HEADER_SIZE + ICMP_HEADER_SIZE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IcmpType {
    EchoReply = 0,
    DestUnreachable = 3,
    Redirect = 5,
    EchoRequest = 8,
    TimeExceeded = 11,
}

impl IcmpType {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::EchoReply),
            3 => Some(Self::DestUnreachable),
            5 => Some(Self::Redirect),
            8 => Some(Self::EchoRequest),
            11 => Some(Self::TimeExceeded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum UnreachableCode {
    Network = 0,
    Host = 1,
    Protocol = 2,
    Port = 3,
    /// Fragmentation needed but DF set.
    FragNeeded = 4,
}

impl UnreachableCode {
    #[allow(dead_code)]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Network),
            1 => Some(Self::Host),
            2 => Some(Self::Protocol),
            3 => Some(Self::Port),
            4 => Some(Self::FragNeeded),
            _ => None,
        }
    }
}

pub fn build_ip_header(
    buffer: &mut [u8],
    src: Ipv4Addr,
    dst: Ipv4Addr,
    total_len: u16,
    ttl: u8,
    protocol: u8,
    id: u16,
) -> usize {
    if buffer.len() < IP_HEADER_SIZE {
        return 0;
    }

    // IPv4, IHL=5 (20 bytes).
    buffer[0] = 0x45;
    buffer[1] = 0x00;
    buffer[2..4].copy_from_slice(&total_len.to_be_bytes());
    buffer[4..6].copy_from_slice(&id.to_be_bytes());
    // DF=1.
    buffer[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    buffer[8] = ttl;
    buffer[9] = protocol;
    buffer[10] = 0;
    buffer[11] = 0;
    buffer[12..16].copy_from_slice(src.as_bytes());
    buffer[16..20].copy_from_slice(dst.as_bytes());

    let checksum = calculate_checksum(&buffer[0..IP_HEADER_SIZE]);
    buffer[10..12].copy_from_slice(&checksum.to_be_bytes());

    IP_HEADER_SIZE
}

pub fn build_icmp_echo_request(
    buffer: &mut [u8],
    id: u16,
    sequence: u16,
    payload_size: usize,
) -> usize {
    let total_size = ICMP_HEADER_SIZE + payload_size;

    if buffer.len() < total_size {
        return 0;
    }

    buffer[0] = IcmpType::EchoRequest as u8;
    buffer[1] = 0;
    buffer[2] = 0;
    buffer[3] = 0;
    buffer[4..6].copy_from_slice(&id.to_be_bytes());
    buffer[6..8].copy_from_slice(&sequence.to_be_bytes());

    for i in 0..payload_size {
        buffer[ICMP_HEADER_SIZE + i] = (i & 0xFF) as u8;
    }

    let checksum = calculate_checksum(&buffer[0..total_size]);
    buffer[2..4].copy_from_slice(&checksum.to_be_bytes());

    total_size
}

#[derive(Debug, Clone, Copy)]
pub struct ParsedIcmpReply {
    pub icmp_type: u8,
    pub icmp_code: u8,
    pub id: u16,
    pub sequence: u16,
    pub src_ip: Ipv4Addr,
    pub ttl: u8,
}

/// Returns `(ihl, protocol, ttl, src, dst)`.
pub fn parse_ip_header(data: &[u8]) -> Option<(usize, u8, u8, Ipv4Addr, Ipv4Addr)> {
    if data.len() < IP_HEADER_SIZE {
        return None;
    }

    let version_ihl = data[0];
    let version = version_ihl >> 4;
    let ihl = (version_ihl & 0x0F) as usize * 4;

    if version != 4 || ihl < IP_HEADER_SIZE || data.len() < ihl {
        return None;
    }

    let protocol = data[9];
    let ttl = data[8];
    let src_ip = Ipv4Addr::new(data[12], data[13], data[14], data[15]);
    let dst_ip = Ipv4Addr::new(data[16], data[17], data[18], data[19]);

    Some((ihl, protocol, ttl, src_ip, dst_ip))
}

pub fn parse_icmp_reply(data: &[u8]) -> Option<ParsedIcmpReply> {
    let (ihl, protocol, ttl, src_ip, _dst_ip) = parse_ip_header(data)?;

    if protocol != ICMP_PROTOCOL {
        return None;
    }

    if data.len() < ihl + ICMP_HEADER_SIZE {
        return None;
    }

    let icmp = &data[ihl..];
    let icmp_type = icmp[0];
    let icmp_code = icmp[1];
    let id = u16::from_be_bytes([icmp[4], icmp[5]]);
    let sequence = u16::from_be_bytes([icmp[6], icmp[7]]);

    Some(ParsedIcmpReply {
        icmp_type,
        icmp_code,
        id,
        sequence,
        src_ip,
        ttl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ip_header() {
        let mut buffer = [0u8; 32];
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::new(1, 1, 1, 1);

        let len = build_ip_header(&mut buffer, src, dst, 60, 64, ICMP_PROTOCOL, 0x1234);

        assert_eq!(len, 20);
        assert_eq!(buffer[0], 0x45);
        assert_eq!(buffer[8], 64);
        assert_eq!(buffer[9], ICMP_PROTOCOL);
    }

    #[test]
    fn test_build_icmp_echo_request() {
        let mut buffer = [0u8; 64];

        let len = build_icmp_echo_request(&mut buffer, 0x1234, 1, 32);

        assert_eq!(len, 8 + 32);
        assert_eq!(buffer[0], IcmpType::EchoRequest as u8);
        assert_eq!(buffer[1], 0);
    }

    #[test]
    fn test_parse_ip_header() {
        let mut data = [0u8; 32];
        let src = Ipv4Addr::new(192, 168, 1, 100);
        let dst = Ipv4Addr::new(1, 1, 1, 1);

        build_ip_header(&mut data, src, dst, 60, 64, ICMP_PROTOCOL, 0x1234);

        let (ihl, protocol, ttl, parsed_src, parsed_dst) = parse_ip_header(&data).unwrap();

        assert_eq!(ihl, 20);
        assert_eq!(protocol, ICMP_PROTOCOL);
        assert_eq!(ttl, 64);
        assert_eq!(parsed_src, src);
        assert_eq!(parsed_dst, dst);
    }

    #[test]
    fn test_icmp_type_from_u8() {
        assert_eq!(IcmpType::from_u8(0), Some(IcmpType::EchoReply));
        assert_eq!(IcmpType::from_u8(8), Some(IcmpType::EchoRequest));
        assert_eq!(IcmpType::from_u8(99), None);
    }
}
