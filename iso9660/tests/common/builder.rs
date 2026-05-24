use crate::common::MemoryBlockDevice;
use std::collections::HashMap;

#[allow(dead_code)]
pub struct IsoBuilder {
    files: HashMap<String, Vec<u8>>,
    pvd_lba: u32,
    root_lba: u32,
    next_free_lba: u32,
}

#[allow(dead_code)]
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
        let mut max_lba = self.next_free_lba;
        let mut file_lbas = HashMap::new();

        for (name, content) in &self.files {
            file_lbas.insert(name.clone(), max_lba);
            let sectors = (content.len() + 2047) / 2048;
            max_lba += sectors as u32;
        }

        let mut data = vec![0u8; (max_lba as usize + 1) * 2048];

        // PVD at LBA 16 (ISO 9660 §8.4).
        let pvd_offset = 16 * 2048;
        data[pvd_offset] = 1;
        data[pvd_offset + 1..pvd_offset + 6].copy_from_slice(b"CD001");
        data[pvd_offset + 6] = 1;

        // Root dir record: 33 fixed bytes + 1-byte name.
        let root_entry_len = 34;
        data[pvd_offset + 156] = root_entry_len;
        Self::write_both_endian_u32(&mut data[pvd_offset + 158..], self.root_lba);
        Self::write_both_endian_u32(&mut data[pvd_offset + 166..], 2048);

        data[pvd_offset + 181] = 0x02; // dir flag
        data[pvd_offset + 188] = 1;
        data[pvd_offset + 189] = 0; // name "."

        Self::write_both_endian_u32(&mut data[pvd_offset + 80..], max_lba);
        Self::write_both_endian_u16(&mut data[pvd_offset + 128..], 2048);

        // Terminator at LBA 17.
        let term_offset = 17 * 2048;
        data[term_offset] = 255;
        data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
        data[term_offset + 6] = 1;

        // Root dir at LBA 18: "." and ".." entries, then files.
        let root_offset = self.root_lba as usize * 2048;
        let mut dir_offset = root_offset;

        Self::write_dir_entry(&mut data, &mut dir_offset, self.root_lba, 2048, 0x02, "\0");
        Self::write_dir_entry(
            &mut data,
            &mut dir_offset,
            self.root_lba,
            2048,
            0x02,
            "\x01",
        );

        for (name, content) in &self.files {
            let lba = file_lbas[name];
            let size = content.len() as u32;
            Self::write_dir_entry(&mut data, &mut dir_offset, lba, size, 0x00, name);

            let file_offset = lba as usize * 2048;
            data[file_offset..file_offset + content.len()].copy_from_slice(content);
        }

        MemoryBlockDevice::new(data)
    }

    // ISO 9660 numeric fields are stored both LSB-first and MSB-first.
    fn write_both_endian_u32(dst: &mut [u8], value: u32) {
        dst[0..4].copy_from_slice(&value.to_le_bytes());
        dst[4..8].copy_from_slice(&value.to_be_bytes());
    }

    fn write_both_endian_u16(dst: &mut [u8], value: u16) {
        dst[0..2].copy_from_slice(&value.to_le_bytes());
        dst[2..4].copy_from_slice(&value.to_be_bytes());
    }

    fn write_dir_entry(
        data: &mut [u8],
        offset: &mut usize,
        lba: u32,
        size: u32,
        flags: u8,
        name: &str,
    ) {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len();
        let mut entry_len = 33 + name_len;
        if entry_len % 2 != 0 {
            entry_len += 1;
        } // Padding to even

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
