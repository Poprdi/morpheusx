//! GPT partition operations for post-EBS.
//!
//! Allocation-free GPT manipulation using only stack buffers.
//! All operations work with the VirtioBlkBlockIo adapter.

use gpt_disk_io::BlockIo;
use gpt_disk_types::{Lba, LbaLe};

use super::types::{guid, DiskError, DiskResult, PartitionInfo, SECTOR_SIZE};

/// GPT header constants
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const GPT_REVISION: u32 = 0x00010000;
const PARTITION_ENTRY_SIZE: usize = 128;
const MAX_PARTITION_ENTRIES: usize = 128;

/// GPT operations helper
pub struct GptOps;

impl GptOps {
    /// Scan disk for existing partitions
    ///
    /// Returns array of partition infos and count of valid partitions.
    pub fn scan_partitions<B: BlockIo>(
        block_io: &mut B,
    ) -> DiskResult<([PartitionInfo; 16], usize)> {
        let mut partitions = [PartitionInfo::default(); 16];
        let mut count = 0;

        // Read GPT header (LBA 1)
        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        // Validate signature
        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        // Get partition entry LBA and count
        let entry_lba = u64::from_le_bytes(header_buf[72..80].try_into().unwrap());
        let num_entries = u32::from_le_bytes(header_buf[80..84].try_into().unwrap()) as usize;
        let entry_size = u32::from_le_bytes(header_buf[84..88].try_into().unwrap()) as usize;

        if entry_size != PARTITION_ENTRY_SIZE {
            return Err(DiskError::InvalidGpt);
        }

        // Read partition entries (32 sectors for 128 entries)
        let mut entry_buf = [0u8; SECTOR_SIZE * 32];
        for i in 0..32 {
            let sector_buf = &mut entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .read_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        // Parse entries
        let entries_to_check = num_entries.min(MAX_PARTITION_ENTRIES);
        for i in 0..entries_to_check {
            let offset = i * PARTITION_ENTRY_SIZE;
            let entry = &entry_buf[offset..offset + PARTITION_ENTRY_SIZE];

            // Check if entry is used (type GUID not zero)
            let type_guid: [u8; 16] = entry[0..16].try_into().unwrap();
            if type_guid == [0u8; 16] {
                continue;
            }

            if count >= 16 {
                break; // Max partitions we track
            }

            let start_lba = u64::from_le_bytes(entry[32..40].try_into().unwrap());
            let end_lba = u64::from_le_bytes(entry[40..48].try_into().unwrap());

            partitions[count] = PartitionInfo::new(i as u8, start_lba, end_lba, type_guid);

            // Copy name (UTF-16LE to ASCII)
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

    /// Find contiguous free space on disk
    ///
    /// Returns (start_lba, end_lba) of largest free region.
    pub fn find_free_space<B: BlockIo>(block_io: &mut B) -> DiskResult<(u64, u64)> {
        // Read GPT header
        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let first_usable = u64::from_le_bytes(header_buf[40..48].try_into().unwrap());
        let last_usable = u64::from_le_bytes(header_buf[48..56].try_into().unwrap());

        // Scan partitions
        let (partitions, count) = Self::scan_partitions(block_io)?;

        if count == 0 {
            return Ok((first_usable, last_usable));
        }

        // Sort partitions by start LBA (simple bubble sort, count is small)
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

        // Find largest gap
        let mut best_start = 0u64;
        let mut best_size = 0u64;

        // Gap before first partition
        if sorted[0].0 > first_usable {
            let gap_size = sorted[0].0 - first_usable;
            if gap_size > best_size {
                best_start = first_usable;
                best_size = gap_size;
            }
        }

        // Gaps between partitions
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

        // Gap after last partition
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

    /// Create a new partition
    ///
    /// Finds free slot in GPT and writes partition entry.
    pub fn create_partition<B: BlockIo>(
        block_io: &mut B,
        start_lba: u64,
        end_lba: u64,
        type_guid: [u8; 16],
        name: &str,
    ) -> DiskResult<u8> {
        // Read GPT header
        let mut header_buf = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(Lba(1), &mut header_buf)
            .map_err(|_| DiskError::IoError)?;

        if &header_buf[0..8] != GPT_SIGNATURE {
            return Err(DiskError::InvalidGpt);
        }

        let entry_lba = u64::from_le_bytes(header_buf[72..80].try_into().unwrap());

        // Read partition entries
        let mut entry_buf = [0u8; SECTOR_SIZE * 32];
        for i in 0..32 {
            let sector_buf = &mut entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .read_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        // Find empty slot
        let mut slot_index = None;
        for i in 0..MAX_PARTITION_ENTRIES {
            let offset = i * PARTITION_ENTRY_SIZE;
            let type_bytes = &entry_buf[offset..offset + 16];
            if type_bytes == &[0u8; 16] {
                slot_index = Some(i);
                break;
            }
        }

        let slot = slot_index.ok_or(DiskError::NoFreeSpace)?;
        let offset = slot * PARTITION_ENTRY_SIZE;

        // Build partition entry
        entry_buf[offset..offset + 16].copy_from_slice(&type_guid);

        // Unique GUID (simple non-zero value)
        entry_buf[offset + 16] = (slot + 1) as u8;
        entry_buf[offset + 17] = 0x12;
        entry_buf[offset + 18] = 0x34;
        entry_buf[offset + 19] = 0x56;

        // LBAs
        entry_buf[offset + 32..offset + 40].copy_from_slice(&start_lba.to_le_bytes());
        entry_buf[offset + 40..offset + 48].copy_from_slice(&end_lba.to_le_bytes());

        // Name (UTF-16LE)
        let name_bytes = name.as_bytes();
        for (i, &b) in name_bytes.iter().take(36).enumerate() {
            entry_buf[offset + 56 + i * 2] = b;
            entry_buf[offset + 56 + i * 2 + 1] = 0;
        }

        // Calculate new CRC32 for partition array
        let array_crc = crc32(&entry_buf[..MAX_PARTITION_ENTRIES * PARTITION_ENTRY_SIZE]);
        header_buf[88..92].copy_from_slice(&array_crc.to_le_bytes());

        // Recalculate header CRC32
        header_buf[16..20].fill(0); // Zero CRC field first
        let header_crc = crc32(&header_buf[0..92]);
        header_buf[16..20].copy_from_slice(&header_crc.to_le_bytes());

        // Write back partition entries
        for i in 0..32 {
            let sector_buf = &entry_buf[i * SECTOR_SIZE..(i + 1) * SECTOR_SIZE];
            block_io
                .write_blocks(Lba(entry_lba + i as u64), sector_buf)
                .map_err(|_| DiskError::IoError)?;
        }

        // Write back header
        block_io
            .write_blocks(Lba(1), &header_buf)
            .map_err(|_| DiskError::IoError)?;

        block_io.flush().map_err(|_| DiskError::IoError)?;

        Ok(slot as u8)
    }
}

/// CRC32 (IEEE 802.3 polynomial) - allocation-free implementation
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
