//! El Torito 16-bit wrapping checksum.

/// Wrapping sum of little-endian 16-bit words. Trailing odd byte ignored.
pub fn checksum_16(data: &[u8]) -> u16 {
    let mut sum = 0u16;
    for chunk in data.chunks_exact(2) {
        let word = u16::from_le_bytes([chunk[0], chunk[1]]);
        sum = sum.wrapping_add(word);
    }
    sum
}

/// El Torito validation passes when the running sum is zero.
pub fn verify_checksum_16(data: &[u8]) -> bool {
    checksum_16(data) == 0
}

/// Value that, when added, makes the total sum zero.
pub fn calculate_complement_16(data: &[u8]) -> u16 {
    let sum = checksum_16(data);
    0u16.wrapping_sub(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum_16() {
        let data = [0x01, 0x00, 0x02, 0x00];
        assert_eq!(checksum_16(&data), 0x0003);
    }

    #[test]
    fn test_verify_checksum() {
        let data = [0x01, 0x00, 0xFF, 0xFF];
        assert_eq!(checksum_16(&data), 0x0000);
        assert!(verify_checksum_16(&data));
    }
}
