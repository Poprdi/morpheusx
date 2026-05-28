//! 8.3 filename helpers. CRC32-hashed manifest names avoid truncation collisions.

extern crate alloc;
use alloc::format;
use alloc::string::String;

/// "<CRC32(iso_name) as 8 hex>.MFS".
pub fn generate_8_3_manifest_name(iso_name: &str) -> String {
    let hash = crc32(iso_name.as_bytes());
    format!("{:08X}.MFS", hash)
}

/// Standard CRC32, polynomial 0xEDB88320.
fn crc32(data: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0xEDB88320;

    let mut crc: u32 = 0xFFFFFFFF;

    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLYNOMIAL;
            } else {
                crc >>= 1;
            }
        }
    }

    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn test_crc32_known_values() {
        assert_eq!(crc32(b""), 0x00000000);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414FA339
        );
    }

    #[test]
    fn test_generate_8_3_manifest_name_format() {
        let result = generate_8_3_manifest_name("tails-6.10.iso");
        assert_eq!(result.len(), 12);
        assert!(result.ends_with(".MFS"));
        let name_part = &result[..8];
        for c in name_part.chars() {
            assert!(matches!(c, '0'..='9' | 'A'..='F'));
        }
    }

    #[test]
    fn test_different_names_different_hashes() {
        let hash1 = generate_8_3_manifest_name("tails-6.10.iso");
        let hash2 = generate_8_3_manifest_name("ubuntu-24.04-desktop.iso");
        let hash3 = generate_8_3_manifest_name("kali-2024.4.iso");
        assert_ne!(hash1, hash2);
        assert_ne!(hash2, hash3);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_same_name_same_hash() {
        let hash1 = generate_8_3_manifest_name("test.iso");
        let hash2 = generate_8_3_manifest_name("test.iso");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_collision_resistance() {
        let distros = [
            "tails-6.10.iso",
            "ubuntu-24.04-desktop.iso",
            "ubuntu-24.04-server.iso",
            "fedora-41-workstation.iso",
            "kali-2024.4.iso",
            "debian-12.8-netinst.iso",
            "arch-linux-latest.iso",
            "linuxmint-22.iso",
        ];

        let mut hashes = Vec::new();
        for distro in &distros {
            let hash = generate_8_3_manifest_name(distro);
            assert!(!hashes.contains(&hash), "Collision detected for {}", distro);
            hashes.push(hash);
        }
    }
}
