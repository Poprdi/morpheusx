//! Stack-buffered GPT manipulation. UEFI 2.10 §5.3.

use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

use super::types::{DiskError, DiskResult, PartitionInfo, SECTOR_SIZE};

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
#[allow(dead_code)]
const GPT_REVISION: u32 = 0x00010000;
const PARTITION_ENTRY_SIZE: usize = 128;
const MAX_PARTITION_ENTRIES: usize = 128;

pub struct GptOps;

impl GptOps {
    pub fn scan_partitions<B: BlockIo>(
        block_io: &mut B,
    ) -> DiskResult<([PartitionInfo; 16], usize)> {
        let mut partitions = [PartitionInfo::default(); 16];
        let mut count = 0;

        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let entry_lba = u64::from_le_bytes(header_buf[72..80].try_into().unwrap());
        let num_entries = u32::from_le_bytes(header_buf[80..84].try_into().unwrap()) as usize;
        let entry_size = u32::from_le_bytes(header_buf[84..88].try_into().unwrap()) as usize;

        if entry_size != PARTITION_ENTRY_SIZE {
            return Err(DiskError::InvalidGpt);
        }

        // 128 entries × 128 B = 32 sectors.
        let mut entry_buf = [0u8; SECTOR_SIZE * 32];
        for i in 0..32 {
            let sector_buf = &mut entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .read_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        let entries_to_check = num_entries.min(MAX_PARTITION_ENTRIES);
        for i in 0..entries_to_check {
            let offset = i * PARTITION_ENTRY_SIZE;
            let entry = &entry_buf[offset..offset + PARTITION_ENTRY_SIZE];

            let type_guid: [u8; 16] = entry[0..16].try_into().unwrap();
            if type_guid == [0u8; 16] {
                continue;
            }

            if count >= 16 {
                break;
            }

            let start_lba = u64::from_le_bytes(entry[32..40].try_into().unwrap());
            let end_lba = u64::from_le_bytes(entry[40..48].try_into().unwrap());

            partitions[count] = PartitionInfo::new(i as u8, start_lba, end_lba, type_guid);

            // UTF-16LE → ASCII (low byte only).
            for j in 0..36 {
                let utf16_offset = 56 + j * 2;
                if utf16_offset < PARTITION_ENTRY_SIZE {
                    partitions[count].name[j] = entry[utf16_offset];
                }
            }

            count += 1;
        }

        Ok((partitions, count))
    }

    /// Returns (start, end) of the largest free LBA range.
    pub fn find_free_space<B: BlockIo>(block_io: &mut B) -> DiskResult<(u64, u64)> {
        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let first_usable = u64::from_le_bytes(header_buf[40..48].try_into().unwrap());
        let last_usable = u64::from_le_bytes(header_buf[48..56].try_into().unwrap());

        let (partitions, count) = Self::scan_partitions(block_io)?;

        if count == 0 {
            return Ok((first_usable, last_usable));
        }

        // count <= 16; bubble sort by start LBA.
        let mut sorted: [(u64, u64); 16] = [(0, 0); 16];
        for i in 0..count {
            sorted[i] = (partitions[i].start_lba, partitions[i].end_lba);
        }
        for _ in 0..count {
            for j in 0..count.saturating_sub(1) {
                if sorted[j].0 > sorted[j + 1].0 {
                    sorted.swap(j, j + 1);
                }
            }
        }

        let mut best_start = 0u64;
        let mut best_size = 0u64;

        if sorted[0].0 > first_usable {
            let gap_size = sorted[0].0 - first_usable;
            if gap_size > best_size {
                best_start = first_usable;
                best_size = gap_size;
            }
        }

        for i in 0..count.saturating_sub(1) {
            let gap_start = sorted[i].1 + 1;
            let gap_end = sorted[i + 1].0.saturating_sub(1);
            if gap_end > gap_start {
                let gap_size = gap_end - gap_start + 1;
                if gap_size > best_size {
                    best_start = gap_start;
                    best_size = gap_size;
                }
            }
        }

        if sorted[count - 1].1 < last_usable {
            let gap_start = sorted[count - 1].1 + 1;
            let gap_size = last_usable - gap_start + 1;
            if gap_size > best_size {
                best_start = gap_start;
                best_size = gap_size;
            }
        }

        if best_size == 0 {
            return Err(DiskError::NoFreeSpace);
        }

        Ok((best_start, best_start + best_size - 1))
    }

    /// Ok(true) when [start,end] doesn't overlap any partition and fits in usable area.
    pub fn verify_range_free<B: BlockIo>(
        block_io: &mut B,
        start_lba: u64,
        end_lba: u64,
    ) -> DiskResult<bool> {
        let (partitions, count) = Self::scan_partitions(block_io)?;

        for part in partitions.iter().take(count) {
            if start_lba <= part.end_lba && part.start_lba <= end_lba {
                return Ok(false);
            }
        }

        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let first_usable = u64::from_le_bytes(header_buf[40..48].try_into().unwrap());
        let last_usable = u64::from_le_bytes(header_buf[48..56].try_into().unwrap());

        if start_lba < first_usable || end_lba > last_usable {
            return Ok(false);
        }

        Ok(true)
    }

    /// Writes the partition entry, then both primary + backup headers/arrays.
    pub fn create_partition<B: BlockIo>(
        block_io: &mut B,
        start_lba: u64,
        end_lba: u64,
        type_guid: [u8; 16],
        name: &str,
    ) -> DiskResult<u8> {
        let mut primary_header = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut primary_header)
            .map_err(|_| DiskError::IoError)?;

        if &primary_header[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let my_lba = u64::from_le_bytes(primary_header[24..32].try_into().unwrap());
        let alternate_lba = u64::from_le_bytes(primary_header[32..40].try_into().unwrap());
        let entry_lba = u64::from_le_bytes(primary_header[72..80].try_into().unwrap());
        let _num_entries = u32::from_le_bytes(primary_header[80..84].try_into().unwrap());
        let _entry_size = u32::from_le_bytes(primary_header[84..88].try_into().unwrap());

        let mut entry_buf = [0u8; SECTOR_SIZE * 32];
        for i in 0..32 {
            let sector_buf = &mut entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .read_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        let mut slot_index = None;
        for i in 0..MAX_PARTITION_ENTRIES {
            let offset = i * PARTITION_ENTRY_SIZE;
            let type_bytes = &entry_buf[offset..offset + 16];
            if type_bytes == [0u8; 16] {
                slot_index = Some(i);
                break;
            }
        }

        let slot = slot_index.ok_or(DiskError::NoFreeSpace)?;
        let offset = slot * PARTITION_ENTRY_SIZE;

        entry_buf[offset..offset + 16].copy_from_slice(&type_guid);

        // Non-zero unique GUID; doesn't need to be globally unique here.
        entry_buf[offset + 16] = (slot + 1) as u8;
        entry_buf[offset + 17] = 0x12;
        entry_buf[offset + 18] = 0x34;
        entry_buf[offset + 19] = 0x56;

        entry_buf[offset + 32..offset + 40].copy_from_slice(&start_lba.to_le_bytes());
        entry_buf[offset + 40..offset + 48].copy_from_slice(&end_lba.to_le_bytes());

        entry_buf[offset + 48..offset + 56].fill(0);

        // Name as UTF-16LE.
        let name_bytes = name.as_bytes();
        for (i, &b) in name_bytes.iter().take(36).enumerate() {
            entry_buf[offset + 56 + i * 2] = b;
            entry_buf[offset + 56 + i * 2 + 1] = 0;
        }

        let array_crc = crc32(&entry_buf[..MAX_PARTITION_ENTRIES * PARTITION_ENTRY_SIZE]);

        primary_header[88..92].copy_from_slice(&array_crc.to_le_bytes());

        primary_header[16..20].fill(0);
        let primary_header_crc = crc32(&primary_header[0..92]);
        primary_header[16..20].copy_from_slice(&primary_header_crc.to_le_bytes());

        for i in 0..32 {
            let sector_buf = &entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .write_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        block_io
            .write_blocks(Lba(1), &primary_header)
            .map_err(|_| DiskError::IoError)?;

        // Backup entry array lives in the 32 sectors before the backup header.
        let backup_entries_lba = alternate_lba - 32;

        for i in 0..32 {
            let sector_buf = &entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .write_blocks(Lba(backup_entries_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        // Backup header: primary with swapped my_lba/alternate_lba.
        let mut backup_header = primary_header;
        backup_header[24..32].copy_from_slice(&alternate_lba.to_le_bytes());
        backup_header[32..40].copy_from_slice(&my_lba.to_le_bytes());
        backup_header[72..80].copy_from_slice(&backup_entries_lba.to_le_bytes());
        backup_header[88..92].copy_from_slice(&array_crc.to_le_bytes());

        backup_header[16..20].fill(0);
        let backup_header_crc = crc32(&backup_header[0..92]);
        backup_header[16..20].copy_from_slice(&backup_header_crc.to_le_bytes());

        block_io
            .write_blocks(Lba(alternate_lba), &backup_header)
            .map_err(|_| DiskError::IoError)?;

        block_io.flush().map_err(|_| DiskError::IoError)?;

        Ok(slot as u8)
    }
}

/// CRC32 IEEE 802.3.
fn crc32(data: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0xEDB88320;
    let mut crc = 0xFFFF_FFFFu32;

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
