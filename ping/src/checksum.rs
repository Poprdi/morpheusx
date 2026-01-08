//! Internet Checksum Implementation (RFC 1071)

/// Calculate Internet checksum for IP/ICMP packets
///
/// Implements the one's complement sum algorithm as specified in RFC 1071.
pub fn calculate_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    // Sum all 16-bit words
    while i + 1 < data.len() {
        let word = u16::from_be_bytes([data[i], data[i + 1]]);
        sum = sum.wrapping_add(word as u32);
        i += 2;
    }

    // Handle odd byte (pad with zero)
    if i < data.len() {
        sum = sum.wrapping_add((data[i] as u32) << 8);
    }

    // Fold 32-bit sum to 16-bit with carry
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    // One's complement
    !(sum as u16)
}

/// Verify checksum of a packet
///
/// Returns true if checksum is valid (result is zero when computed over
/// data including the checksum field).
pub fn verify_checksum(data: &[u8]) -> bool {
    calculate_checksum(data) == 0
}

/// Calculate partial checksum (for incremental computation)
pub fn partial_checksum(data: &[u8], initial: u32) -> u32 {
    let mut sum = initial;
    let mut i = 0;

    while i + 1 < data.len() {
        let word = u16::from_be_bytes([data[i], data[i + 1]]);
        sum = sum.wrapping_add(word as u32);
        i += 2;
    }

    if i < data.len() {
        sum = sum.wrapping_add((data[i] as u32) << 8);
    }

    sum
}

/// Finalize a partial checksum
pub fn finalize_checksum(sum: u32) -> u16 {
    let mut s = sum;
    while s >> 16 != 0 {
        s = (s & 0xFFFF) + (s >> 16);
    }
    !(s as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_zeros() {
        // All zeros should give checksum of 0xFFFF
        let data = [0u8; 20];
        assert_eq!(calculate_checksum(&data), 0xFFFF);
    }

    #[test]
    fn test_checksum_ones() {
        // All 0xFF should fold to 0
        let data = [0xFFu8; 20];
        assert_eq!(calculate_checksum(&data), 0);
    }

    #[test]
    fn test_verify_valid() {
        // Create data with valid checksum
        let mut data = [0x45, 0x00, 0x00, 0x3c, 0x1c, 0x46, 0x40, 0x00,
                        0x40, 0x06, 0x00, 0x00, 0xac, 0x10, 0x0a, 0x63,
                        0xac, 0x10, 0x0a, 0x0c];
        
        // Calculate and insert checksum
        let checksum = calculate_checksum(&data);
        data[10] = (checksum >> 8) as u8;
        data[11] = (checksum & 0xFF) as u8;
        
        // Verify should now pass
        assert!(verify_checksum(&data));
    }

    #[test]
    fn test_odd_length() {
        // Odd length data
        let data = [0x45, 0x00, 0x00];
        let _ = calculate_checksum(&data); // Should not panic
    }

    #[test]
    fn test_partial_checksum() {
        let data = [0x45, 0x00, 0x00, 0x3c];
        let partial = partial_checksum(&data, 0);
        let full = finalize_checksum(partial);
        
        // Should match direct calculation
        assert_eq!(full, calculate_checksum(&data));
    }
}
