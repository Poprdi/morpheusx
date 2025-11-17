// FAT32 directory entry types

extern crate alloc;
use alloc::vec::Vec;

const SECTOR_SIZE: usize = 512;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE: u8 = 0x20;

/// FAT32 directory entry (32 bytes)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 11], // 8.3 filename
    pub attr: u8,       // File attributes
    pub _reserved: u8,
    pub _create_time_tenth: u8,
    pub _create_time: u16,
    pub _create_date: u16,
    pub _access_date: u16,
    pub cluster_high: u16, // High word of first cluster
    pub _modify_time: u16,
    pub _modify_date: u16,
    pub cluster_low: u16, // Low word of first cluster
    pub file_size: u32,   // File size in bytes
}

impl DirEntry {
    pub fn empty() -> Self {
        Self {
            name: [0; 11],
            attr: 0,
            _reserved: 0,
            _create_time_tenth: 0,
            _create_time: 0,
            _create_date: 0,
            _access_date: 0,
            cluster_high: 0,
            _modify_time: 0,
            _modify_date: 0,
            cluster_low: 0,
            file_size: 0,
        }
    }

    pub fn is_free(&self) -> bool {
        self.name[0] == 0x00 || self.name[0] == 0xE5
    }

    pub fn set_name(&mut self, name: &str) {
        // Convert to 8.3 format (simple, no LFN)
        self.name = [0x20; 11]; // Fill with spaces

        let parts: Vec<&str> = name.split('.').collect();
        let basename = parts[0].as_bytes();
        let ext = if parts.len() > 1 {
            parts[1].as_bytes()
        } else {
            b""
        };

        let base_len = basename.len().min(8);
        self.name[..base_len].copy_from_slice(&basename[..base_len]);

        let ext_len = ext.len().min(3);
        self.name[8..8 + ext_len].copy_from_slice(&ext[..ext_len]);

        // Convert to uppercase
        for byte in &mut self.name {
            if *byte >= b'a' && *byte <= b'z' {
                *byte -= 32;
            }
        }
    }

    pub fn first_cluster(&self) -> u32 {
        ((self.cluster_high as u32) << 16) | (self.cluster_low as u32)
    }

    pub fn set_first_cluster(&mut self, cluster: u32) {
        self.cluster_high = (cluster >> 16) as u16;
        self.cluster_low = (cluster & 0xFFFF) as u16;
    }
}
