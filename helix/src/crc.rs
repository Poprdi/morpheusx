//! Software CRC32C and CRC64 for HelixFS.
//!
//! In the future these can be swapped for hardware `crc32` (SSE4.2) via
//! inline assembly.  The table-driven implementations here are correct
//! reference code that works in any `no_std` environment.

/// CRC32C (Castagnoli) polynomial: 0x1EDC6F41.
const CRC32C_TABLE: [u32; 256] = {
    let poly: u32 = 0x82F6_3B78; // bit-reversed 0x1EDC6F41
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32C of `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

/// Compute CRC32C over two contiguous byte arrays without allocating.
///
/// Equivalent to `crc32c(&[a, b].concat())` but avoids the heap allocation.
pub fn crc32c_two(a: &[u8], b: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in a {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    for &byte in b {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

/// ECMA-182 CRC64 polynomial (used for content dedup).
const CRC64_TABLE: [u64; 256] = {
    let poly: u64 = 0xC96C_5795_D787_0F42;
    let mut table = [0u64; 256];
    let mut i = 0u64;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ poly;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC64 of `data`.
pub fn crc64(data: &[u8]) -> u64 {
    let mut crc: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    for &b in data {
        let idx = ((crc ^ b as u64) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC64_TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF_FFFF_FFFF
}

/// FNV-1a 64-bit hash for path strings.
pub fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01B3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_known_vectors() {
        assert_eq!(crc32c(b""), 0x0000_0000);
        // "123456789" → 0xE3069283 (CRC32C standard test vector)
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn crc64_nonempty() {
        let c = crc64(b"hello");
        assert_ne!(c, 0);
        assert_ne!(c, crc64(b"world"));
    }

    #[test]
    fn fnv1a_stable() {
        let a = fnv1a_64(b"/data/documents/report.pdf");
        let b = fnv1a_64(b"/data/documents/report.pdf");
        assert_eq!(a, b);
        assert_ne!(a, fnv1a_64(b"/data/documents/report.txt"));
    }
}
