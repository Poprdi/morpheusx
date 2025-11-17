// GPT operations using gpt-disk-rs

use crate::disk::partition::{PartitionInfo, PartitionTable, PartitionType};
use gpt_disk_io::{BlockIo, Disk};
use gpt_disk_types::{
    guid, BlockSize, GptHeader, GptPartitionEntry, GptPartitionEntryArray, LbaLe, U32Le,
};

pub enum GptError {
    IoError,
    InvalidHeader,
    NoSpace,
    PartitionNotFound,
    OverlappingPartitions,
    InvalidSize,
    AlignmentError,
}

/// Scan disk for GPT and populate partition table
pub fn scan_partitions<B: BlockIo>(
    block_io: B,
    partition_table: &mut PartitionTable,
    block_size_bytes: usize,
) -> Result<(), GptError> {
    partition_table.clear();

    // Create disk handle - if this fails, the disk may be inaccessible
    let mut disk = match Disk::new(block_io) {
        Ok(d) => d,
        Err(_) => {
            // Can't create disk handle - treat as no GPT
            partition_table.has_gpt = false;
            return Ok(());
        }
    };

    // Try to read GPT header
    let header = match disk.read_primary_gpt_header(&mut [0u8; 512]) {
        Ok(h) => h,
        Err(_) => {
            partition_table.has_gpt = false;
            return Ok(()); // No GPT is not an error
        }
    };

    partition_table.has_gpt = true;

    // Get partition entry layout
    let layout = match header.get_partition_entry_array_layout() {
        Ok(l) => l,
        Err(_) => {
            // Invalid header - treat as no GPT
            partition_table.has_gpt = false;
            return Ok(());
        }
    };

    // Use iterator to read partitions
    let mut entry_buf = [0u8; 4096];
    let entry_buffer = &mut entry_buf[..block_size_bytes];

    let iter = match disk.gpt_partition_entry_array_iter(layout, entry_buffer) {
        Ok(it) => it,
        Err(_) => {
            // Can't read partition array - treat as no GPT
            partition_table.has_gpt = false;
            return Ok(());
        }
    };

    // Populate partition table
    for (index, entry_result) in iter.enumerate() {
        let entry = entry_result.map_err(|_| GptError::IoError)?;

        if !entry.is_used() {
            continue;
        }

        // Copy the guid to avoid unaligned reference
        let guid = entry.partition_type_guid;
        let partition_type = PartitionType::from_gpt_guid(&guid);

        let info = PartitionInfo {
            index: index as u32,
            partition_type,
            start_lba: entry.starting_lba.to_u64(),
            end_lba: entry.ending_lba.to_u64(),
        };

        if partition_table.add_partition(info).is_err() {
            break; // Table full
        }
    }

    Ok(())
}

/// Create fresh GPT on disk
pub fn create_gpt<B: BlockIo>(block_io: B, num_blocks: u64) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    // Create GPT header
    let mut header = GptHeader {
        my_lba: LbaLe::from_u64(1),
        alternate_lba: LbaLe::from_u64(num_blocks - 1),
        first_usable_lba: LbaLe::from_u64(34),
        last_usable_lba: LbaLe::from_u64(num_blocks - 34),
        disk_guid: guid!("12345678-1234-1234-1234-123456789012"),
        partition_entry_lba: LbaLe::from_u64(2),
        number_of_partition_entries: U32Le::from_u32(128),
        ..Default::default()
    };

    header.update_header_crc32();

    // Write protective MBR
    let mut buf = [0u8; 512];
    disk.write_protective_mbr(&mut buf)
        .map_err(|_| GptError::IoError)?;

    // Write primary GPT header
    disk.write_primary_gpt_header(&header, &mut buf)
        .map_err(|_| GptError::IoError)?;

    // Write empty partition array
    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let block_size = BlockSize::BS_512;

    let mut entry_buf = [0u8; 16384];
    let entry_array = GptPartitionEntryArray::new(layout, block_size, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    disk.write_gpt_partition_entry_array(&entry_array)
        .map_err(|_| GptError::IoError)?;

    // Flush to ensure writes are committed
    disk.flush().map_err(|_| GptError::IoError)?;

    Ok(())
}

/// Represents a free space region on disk
#[derive(Copy, Clone, Debug)]
pub struct FreeRegion {
    pub start_lba: u64,
    pub end_lba: u64,
}

impl FreeRegion {
    pub fn size_lba(&self) -> u64 {
        self.end_lba - self.start_lba + 1
    }

    pub fn size_mb(&self) -> u64 {
        (self.size_lba() * 512) / (1024 * 1024)
    }
}

/// Find all free space regions on disk
pub fn find_free_space<B: BlockIo>(
    block_io: B,
    block_size_bytes: usize,
) -> Result<[Option<FreeRegion>; 16], GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    let header = disk
        .read_primary_gpt_header(&mut [0u8; 512])
        .map_err(|_| GptError::InvalidHeader)?;

    let first_usable = header.first_usable_lba.to_u64();
    let last_usable = header.last_usable_lba.to_u64();

    // Get partition layout
    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    // Read partitions to find used ranges
    let mut entry_buf = [0u8; 4096];
    let entry_buffer = &mut entry_buf[..block_size_bytes];

    let iter = disk
        .gpt_partition_entry_array_iter(layout, entry_buffer)
        .map_err(|_| GptError::IoError)?;

    let mut used_ranges: [(u64, u64); 16] = [(0, 0); 16];
    let mut used_count = 0;

    for entry_result in iter {
        let entry = entry_result.map_err(|_| GptError::IoError)?;

        if !entry.is_used() {
            continue;
        }

        if used_count < 16 {
            used_ranges[used_count] = (entry.starting_lba.to_u64(), entry.ending_lba.to_u64());
            used_count += 1;
        }
    }

    let mut regions = [None; 16];
    let mut region_count = 0;

    // Sort by start LBA
    for i in 0..used_count {
        for j in i + 1..used_count {
            if used_ranges[j].0 < used_ranges[i].0 {
                used_ranges.swap(i, j);
            }
        }
    }

    // Find gaps
    let mut current = first_usable;

    for i in 0..used_count {
        let (start, end) = used_ranges[i];

        if current < start && region_count < 16 {
            regions[region_count] = Some(FreeRegion {
                start_lba: current,
                end_lba: start - 1,
            });
            region_count += 1;
        }

        current = end + 1;
    }

    // Add final region if space left
    if current <= last_usable && region_count < 16 {
        regions[region_count] = Some(FreeRegion {
            start_lba: current,
            end_lba: last_usable,
        });
    }

    Ok(regions)
}

///Create a new partition in the specified free region
pub fn create_partition<B: BlockIo>(
    block_io: B,
    partition_type: PartitionType,
    start_lba: u64,
    end_lba: u64,
) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    // Read existing header
    let mut header = disk
        .read_primary_gpt_header(&mut [0u8; 512])
        .map_err(|_| GptError::InvalidHeader)?;

    // Validate range
    let first_usable = header.first_usable_lba.to_u64();
    let last_usable = header.last_usable_lba.to_u64();

    if start_lba < first_usable || end_lba > last_usable || start_lba >= end_lba {
        return Err(GptError::InvalidSize);
    }

    // Read partition array
    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let mut entry_buf = [0u8; 16384];
    let mut entry_array = disk
        .read_gpt_partition_entry_array(layout, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    // Find first empty slot
    let mut slot_index = None;
    let num_entries = layout.num_entries as usize;

    for i in 0..num_entries {
        if let Some(entry) = entry_array.get_partition_entry(i.try_into().unwrap()) {
            if !entry.is_used() {
                slot_index = Some(i);
                break;
            }
        }
    }

    let slot = slot_index.ok_or(GptError::NoSpace)?;

    // Create new entry directly in buffer
    let entry = entry_array
        .get_partition_entry_mut(slot.try_into().unwrap())
        .ok_or(GptError::NoSpace)?;

    entry.partition_type_guid = partition_type.to_gpt_guid();
    entry.unique_partition_guid = guid!("12345678-1234-5678-1234-567812345678"); // TODO: generate unique
    entry.starting_lba = LbaLe::from_u64(start_lba);
    entry.ending_lba = LbaLe::from_u64(end_lba);
    entry.attributes = Default::default();

    // Name is already zeroed in default entry

    // Update CRC in header
    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    // Write everything back
    disk.write_primary_gpt_header(&header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    disk.write_gpt_partition_entry_array(&entry_array)
        .map_err(|_| GptError::IoError)?;

    Ok(())
}

///Delete a partition by index
pub fn delete_partition<B: BlockIo>(block_io: B, partition_index: usize) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    let mut header = disk
        .read_primary_gpt_header(&mut [0u8; 512])
        .map_err(|_| GptError::InvalidHeader)?;

    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let mut entry_buf = [0u8; 16384];
    let mut entry_array = disk
        .read_gpt_partition_entry_array(layout, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    // Clear the entry
    if let Some(entry) = entry_array.get_partition_entry_mut(partition_index.try_into().unwrap()) {
        *entry = GptPartitionEntry::default(); // Zero out the entry
    } else {
        return Err(GptError::PartitionNotFound);
    }

    // Update CRCs
    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    // Write back
    disk.write_primary_gpt_header(&header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    disk.write_gpt_partition_entry_array(&entry_array)
        .map_err(|_| GptError::IoError)?;

    Ok(())
}

/// Shrink a partition to a new smaller size
/// partition_index: GPT entry index (0-127)
/// new_size_mb: New size in megabytes (must be smaller than current)
pub fn shrink_partition<B: BlockIo>(
    block_io: B,
    partition_index: usize,
    new_size_mb: u64,
) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    let mut header = disk
        .read_primary_gpt_header(&mut [0u8; 512])
        .map_err(|_| GptError::InvalidHeader)?;

    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let mut entry_buf = [0u8; 16384];
    let mut entry_array = disk
        .read_gpt_partition_entry_array(layout, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    // Get the partition entry
    let entry = entry_array
        .get_partition_entry_mut(partition_index.try_into().unwrap())
        .ok_or(GptError::PartitionNotFound)?;

    // Check if partition is used
    if !entry.is_used() {
        return Err(GptError::PartitionNotFound);
    }

    let start_lba = entry.starting_lba.to_u64();
    let current_end_lba = entry.ending_lba.to_u64();
    let current_size_lba = current_end_lba - start_lba + 1;

    // Calculate new size in LBA (assume 512-byte blocks)
    let new_size_lba = mb_to_lba(new_size_mb, 512);

    if new_size_lba == 0 {
        return Err(GptError::InvalidSize);
    }

    if new_size_lba >= current_size_lba {
        return Err(GptError::InvalidSize); // Can only shrink
    }

    // Calculate new end LBA
    let new_end_lba = start_lba + new_size_lba - 1;

    // Update the partition entry
    entry.ending_lba = LbaLe::from_u64(new_end_lba);

    // Update CRCs
    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    // Write back
    disk.write_primary_gpt_header(&header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    disk.write_gpt_partition_entry_array(&entry_array)
        .map_err(|_| GptError::IoError)?;

    Ok(())
}

/// Align LBA to 1MB boundary (2048 sectors for 512-byte blocks)
pub fn align_lba(lba: u64, block_size_bytes: u32) -> u64 {
    let alignment = (1024 * 1024) / block_size_bytes as u64; // 1MB alignment
    lba.div_ceil(alignment) * alignment
}

/// Calculate size in LBA from MB
pub fn mb_to_lba(size_mb: u64, block_size_bytes: u32) -> u64 {
    (size_mb * 1024 * 1024) / block_size_bytes as u64
}

/// Calculate total free space on disk in MB
pub fn calculate_total_free_space<B: BlockIo>(
    block_io: B,
    block_size_bytes: usize,
) -> Result<u64, GptError> {
    let free_regions = find_free_space(block_io, block_size_bytes)?;

    let mut total_free_lba = 0u64;
    for region in free_regions.iter().flatten() {
        total_free_lba += region.size_lba();
    }

    // Convert to MB
    Ok((total_free_lba * 512) / (1024 * 1024))
}
