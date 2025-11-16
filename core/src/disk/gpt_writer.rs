// GPT creation and manipulation

use super::gpt::GptHeader;

pub struct PartitionEditor {
    entries: [u8; 16384], // 128 entries x 128 bytes
    modified: bool,
}

impl PartitionEditor {
    pub fn new() -> Self {
        Self {
            entries: [0u8; 16384],
            modified: false,
        }
    }

    pub fn load_from_buffer(&mut self, buffer: &[u8]) {
        let len = buffer.len().min(16384);
        self.entries[..len].copy_from_slice(&buffer[..len]);
        self.modified = false;
    }

    pub fn get_buffer(&self) -> &[u8] {
        &self.entries
    }

    pub fn find_free_slot(&self) -> Option<usize> {
        for i in 0..128 {
            let offset = i * 128;
            let entry_type = &self.entries[offset..offset + 16];
            if entry_type == &[0u8; 16] {
                return Some(i);
            }
        }
        None
    }

    pub fn add_partition(
        &mut self,
        slot: usize,
        type_guid: [u8; 16],
        start_lba: u64,
        end_lba: u64,
        name: &str,
    ) -> Result<(), ()> {
        if slot >= 128 {
            return Err(());
        }

        let entry = create_partition_entry(type_guid, start_lba, end_lba, name);
        let offset = slot * 128;
        self.entries[offset..offset + 128].copy_from_slice(&entry);
        self.modified = true;

        Ok(())
    }

    pub fn delete_partition(&mut self, slot: usize) -> Result<(), ()> {
        if slot >= 128 {
            return Err(());
        }

        let offset = slot * 128;
        self.entries[offset..offset + 128].fill(0);
        self.modified = true;

        Ok(())
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }
}

pub fn find_free_space(partitions: &[(u64, u64)], disk_size_lba: u64) -> Option<(u64, u64)> {
    let first_usable = 34u64;
    let last_usable = disk_size_lba.saturating_sub(34);

    if partitions.is_empty() {
        return Some((first_usable, last_usable));
    }

    // Sort by start LBA
    let mut sorted: [(u64, u64); 16] = [(0, 0); 16];
    let count = partitions.len().min(16);
    sorted[..count].copy_from_slice(&partitions[..count]);

    for _ in 0..count {
        for j in 0..count - 1 {
            if sorted[j].0 > sorted[j + 1].0 {
                sorted.swap(j, j + 1);
            }
        }
    }

    // Check gap before first partition
    if sorted[0].0 > first_usable + 1024 {
        return Some((first_usable, sorted[0].0 - 1));
    }

    // Check gaps between partitions
    for i in 0..count - 1 {
        let gap_start = sorted[i].1 + 1;
        let gap_end = sorted[i + 1].0 - 1;
        if gap_end > gap_start + 1024 {
            return Some((gap_start, gap_end));
        }
    }

    // Check gap after last partition
    if sorted[count - 1].1 < last_usable - 1024 {
        return Some((sorted[count - 1].1 + 1, last_usable));
    }

    None
}

/// Create a blank GPT header for a disk
pub fn create_gpt_header(disk_size_lba: u64) -> GptHeader {
    GptHeader {
        signature: *b"EFI PART",
        revision: 0x00010000, // GPT 1.0
        header_size: 92,
        header_crc32: 0, // Calculate later
        reserved: 0,
        current_lba: 1,
        backup_lba: disk_size_lba - 1,
        first_usable_lba: 34,
        last_usable_lba: disk_size_lba - 34,
        disk_guid: [0u8; 16], // Generate random later
        partition_entry_lba: 2,
        num_partition_entries: 128,
        partition_entry_size: 128,
        partition_array_crc32: 0, // Calculate later
    }
}

/// Write GPT header to a 512-byte buffer
pub fn write_gpt_header(header: &GptHeader, buffer: &mut [u8; 512]) {
    buffer.fill(0);

    // Signature
    buffer[0..8].copy_from_slice(&header.signature);

    // Revision (little-endian u32)
    buffer[8..12].copy_from_slice(&header.revision.to_le_bytes());

    // Header size
    buffer[12..16].copy_from_slice(&header.header_size.to_le_bytes());

    // CRC32 (placeholder)
    buffer[16..20].copy_from_slice(&header.header_crc32.to_le_bytes());

    // Reserved
    buffer[20..24].copy_from_slice(&header.reserved.to_le_bytes());

    // Current LBA
    buffer[24..32].copy_from_slice(&header.current_lba.to_le_bytes());

    // Backup LBA
    buffer[32..40].copy_from_slice(&header.backup_lba.to_le_bytes());

    // First usable LBA
    buffer[40..48].copy_from_slice(&header.first_usable_lba.to_le_bytes());

    // Last usable LBA
    buffer[48..56].copy_from_slice(&header.last_usable_lba.to_le_bytes());

    // Disk GUID
    buffer[56..72].copy_from_slice(&header.disk_guid);

    // Partition entry LBA
    buffer[72..80].copy_from_slice(&header.partition_entry_lba.to_le_bytes());

    // Number of partition entries
    buffer[80..84].copy_from_slice(&header.num_partition_entries.to_le_bytes());

    // Partition entry size
    buffer[84..88].copy_from_slice(&header.partition_entry_size.to_le_bytes());

    // Partition array CRC32
    buffer[88..92].copy_from_slice(&header.partition_array_crc32.to_le_bytes());
}

/// Create a blank partition entry
pub fn create_partition_entry(
    type_guid: [u8; 16],
    start_lba: u64,
    end_lba: u64,
    name: &str,
) -> [u8; 128] {
    let mut entry = [0u8; 128];

    // Type GUID
    entry[0..16].copy_from_slice(&type_guid);

    // Partition GUID (random, for now just use non-zero)
    entry[16] = 1;

    // Starting LBA
    entry[32..40].copy_from_slice(&start_lba.to_le_bytes());

    // Ending LBA
    entry[40..48].copy_from_slice(&end_lba.to_le_bytes());

    // Attributes
    entry[48..56].fill(0);

    // Partition name (UTF-16LE, max 36 characters)
    let name_bytes = name.as_bytes();
    for (i, &byte) in name_bytes.iter().take(36).enumerate() {
        entry[56 + i * 2] = byte;
        entry[56 + i * 2 + 1] = 0; // UTF-16 high byte
    }

    entry
}
