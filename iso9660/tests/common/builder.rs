use crate::common::MemoryBlockDevice;
use std::collections::HashMap;

pub struct IsoBuilder {
    files: HashMap<String, Vec<u8>>,
    pvd_lba: u32,
    root_lba: u32,
    next_free_lba: u32,
}

impl IsoBuilder {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            pvd_lba: 16,
            root_lba: 18,
            next_free_lba: 19, // 16=PVD, 17=Terminator, 18=Root
        }
    }

    pub fn add_file(&mut self, name: &str, content: &[u8]) {
        self.files.insert(name.to_string(), content.to_vec());
    }

    pub fn build(self) -> MemoryBlockDevice {
        // Calculate total size needed
        let mut max_lba = self.next_free_lba;
        let mut file_lbas = HashMap::new();

        // Assign LBAs to files
        for (name, content) in &self.files {
            file_lbas.insert(name.clone(), max_lba);
            let sectors = (content.len() + 2047) / 2048;
            max_lba += sectors as u32;
        }

        // Allocate data
        let mut data = vec![0u8; (max_lba as usize + 1) * 2048];

        // 1. PVD at 16
        let pvd_offset = 16 * 2048;
        data[pvd_offset] = 1; // Primary
        data[pvd_offset + 1..pvd_offset + 6].copy_from_slice(b"CD001");
        data[pvd_offset + 6] = 1; // Version
        
        // Root dir record in PVD
        let root_entry_len = 34; // 33 fixed + 1 name
        data[pvd_offset + 156] = root_entry_len; 
        // Extent LBA (both endian)
        Self::write_both_endian_u32(&mut data[pvd_offset + 158..], self.root_lba);
        // Data Length (both endian) - Root dir size (1 sector for now)
        Self::write_both_endian_u32(&mut data[pvd_offset + 166..], 2048);
        
        data[pvd_offset + 181] = 0x02; // Directory flag
        data[pvd_offset + 188] = 1; // Name len
        data[pvd_offset + 189] = 0; // Name "."

        // Set volume space size (PVD 80)
        Self::write_both_endian_u32(&mut data[pvd_offset + 80..], max_lba);
        // Set logical block size (PVD 128)
        Self::write_both_endian_u16(&mut data[pvd_offset + 128..], 2048);

        // 2. Terminator at 17
        let term_offset = 17 * 2048;
        data[term_offset] = 255;
        data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
        data[term_offset + 6] = 1;

        // 3. Root Directory at 18
        let root_offset = self.root_lba as usize * 2048;
        let mut dir_offset = root_offset;

        // "." entry
        Self::write_dir_entry(&mut data, &mut dir_offset, self.root_lba, 2048, 0x02, "\0");
        // ".." entry
        Self::write_dir_entry(&mut data, &mut dir_offset, self.root_lba, 2048, 0x02, "\x01");

        // File entries
        for (name, content) in &self.files {
            let lba = file_lbas[name];
            let size = content.len() as u32;
            Self::write_dir_entry(&mut data, &mut dir_offset, lba, size, 0x00, name);
            
            // Write file content
            let file_offset = lba as usize * 2048;
            data[file_offset..file_offset + content.len()].copy_from_slice(content);
        }

        MemoryBlockDevice::new(data)
    }

    fn write_both_endian_u32(dst: &mut [u8], value: u32) {
        dst[0..4].copy_from_slice(&value.to_le_bytes());
        dst[4..8].copy_from_slice(&value.to_be_bytes());
    }

    fn write_both_endian_u16(dst: &mut [u8], value: u16) {
        dst[0..2].copy_from_slice(&value.to_le_bytes());
        dst[2..4].copy_from_slice(&value.to_be_bytes());
    }

    fn write_dir_entry(data: &mut [u8], offset: &mut usize, lba: u32, size: u32, flags: u8, name: &str) {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len();
        let mut entry_len = 33 + name_len;
        if entry_len % 2 != 0 { entry_len += 1; } // Padding to even
        
        let start = *offset;
        data[start] = entry_len as u8;
        data[start + 1] = 0; // Ext attr len
        
        Self::write_both_endian_u32(&mut data[start + 2..], lba);
        Self::write_both_endian_u32(&mut data[start + 10..], size);
        
        // Date (7 bytes) - all zero is fine for test
        
        data[start + 25] = flags;
        
        data[start + 28] = 0; // Volume seq
        data[start + 32] = name_len as u8;
        
        data[start + 33..start + 33 + name_len].copy_from_slice(name_bytes);
        
        *offset += entry_len;
    }
}
