//! Table-driven CRC32C / CRC64 / FNV-1a. Could be replaced with SSE4.2 `crc32`.

// slicing-by-4: table[k][b] is the crc of byte b followed by k zero bytes.

/// CRC32C (Castagnoli) polynomial 0x1EDC6F41, bit-reversed.
const CRC32C_TABLE: [[u32; 256]; 4] = {
    let poly: u32 = 0x82F6_3B78;
    let mut t = [[0u32; 256]; 4];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
            j += 1;
        }
        t[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 4 {
        let mut i = 0;
        while i < 256 {
            let prev = t[k - 1][i];
            t[k][i] = (prev >> 8) ^ t[0][(prev & 0xFF) as usize];
            i += 1;
        }
        k += 1;
    }
    t
};

fn crc32c_update(mut crc: u32, data: &[u8]) -> u32 {
    let t = &CRC32C_TABLE;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        crc ^= u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        crc = t[3][(crc & 0xFF) as usize]
            ^ t[2][((crc >> 8) & 0xFF) as usize]
            ^ t[1][((crc >> 16) & 0xFF) as usize]
            ^ t[0][(crc >> 24) as usize];
    }
    for &b in chunks.remainder() {
        crc = (crc >> 8) ^ t[0][((crc ^ b as u32) & 0xFF) as usize];
    }
    crc
}

pub fn crc32c(data: &[u8]) -> u32 {
    crc32c_update(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

/// `crc32c(a ++ b)` without allocating.
pub fn crc32c_two(a: &[u8], b: &[u8]) -> u32 {
    crc32c_update(crc32c_update(0xFFFF_FFFF, a), b) ^ 0xFFFF_FFFF
}

/// ECMA-182 CRC64; used for content checksums.
const CRC64_TABLE: [[u64; 256]; 4] = {
    let poly: u64 = 0xC96C_5795_D787_0F42;
    let mut t = [[0u64; 256]; 4];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u64;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
            j += 1;
        }
        t[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 4 {
        let mut i = 0;
        while i < 256 {
            let prev = t[k - 1][i];
            t[k][i] = (prev >> 8) ^ t[0][(prev & 0xFF) as usize];
            i += 1;
        }
        k += 1;
    }
    t
};

pub fn crc64(data: &[u8]) -> u64 {
    let t = &CRC64_TABLE;
    let mut crc: u64 = 0xFFFF_FFFF_FFFF_FFFF;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        let x = crc ^ u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as u64;
        crc = (crc >> 32)
            ^ t[3][(x & 0xFF) as usize]
            ^ t[2][((x >> 8) & 0xFF) as usize]
            ^ t[1][((x >> 16) & 0xFF) as usize]
            ^ t[0][((x >> 24) & 0xFF) as usize];
    }
    for &b in chunks.remainder() {
        crc = (crc >> 8) ^ t[0][((crc ^ b as u64) & 0xFF) as usize];
    }
    crc ^ 0xFFFF_FFFF_FFFF_FFFF
}

/// FNV-1a 64-bit; used for path hashing.
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
        // Standard test vector: "123456789" → 0xE3069283.
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
