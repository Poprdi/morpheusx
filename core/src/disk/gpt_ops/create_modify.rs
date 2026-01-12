// GPT operations using gpt-disk-rs

use super::{mb_to_lba, GptError};
use crate::disk::partition::PartitionType;
use gpt_disk_io::{BlockIo, Disk};
use gpt_disk_types::{
    guid, BlockSize, GptHeader, GptPartitionEntry, GptPartitionEntryArray,
    GptPartitionEntryArrayLayout, LbaLe, U32Le,
};

/// Helper function to write both primary and secondary GPT headers and partition arrays.
/// This ensures both copies stay in sync to avoid CRC mismatch errors.
fn write_gpt_both<B: BlockIo>(
    disk: &mut Disk<B>,
    header: &mut GptHeader,
    entry_array: &GptPartitionEntryArray,
) -> Result<(), GptError> {
    // Write primary GPT header
    disk.write_primary_gpt_header(header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    // Write primary partition entry array
    disk.write_gpt_partition_entry_array(entry_array)
        .map_err(|_| GptError::IoError)?;

    // Create secondary header (swap my_lba and alternate_lba)
    let mut secondary_header = header.clone();
    let primary_lba = header.my_lba;
    let alternate_lba = header.alternate_lba;

    secondary_header.my_lba = alternate_lba;
    secondary_header.alternate_lba = primary_lba;

    // Secondary partition entry array is before the secondary header
    // Calculate: alternate_lba - (num_entries * entry_size / block_size)
    let num_entries = header.number_of_partition_entries.to_u32() as u64;
    let entry_size = header.size_of_partition_entry.to_u32() as u64;
    let block_size = 512u64; // Assume 512-byte blocks
    let entries_sectors = (num_entries * entry_size + block_size - 1) / block_size;
    let secondary_entry_lba = alternate_lba.to_u64() - entries_sectors;

    secondary_header.partition_entry_lba = LbaLe::from_u64(secondary_entry_lba);

    // Recalculate CRC for secondary header (partition array CRC is the same)
    secondary_header.update_header_crc32();

    // Write secondary GPT header
    disk.write_secondary_gpt_header(&secondary_header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    // Create secondary partition entry array with correct layout
    let secondary_layout = secondary_header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    // Copy primary entry data to a new buffer for secondary
    let primary_storage = entry_array.storage();
    let mut secondary_buf = [0u8; 16384];
    secondary_buf[..primary_storage.len()].copy_from_slice(primary_storage);

    let secondary_entry_array =
        GptPartitionEntryArray::new(secondary_layout, BlockSize::BS_512, &mut secondary_buf)
            .map_err(|_| GptError::IoError)?;

    // Write secondary partition entry array
    disk.write_gpt_partition_entry_array(&secondary_entry_array)
        .map_err(|_| GptError::IoError)?;

    // Flush to ensure all writes are committed
    disk.flush().map_err(|_| GptError::IoError)?;

    Ok(())
}

/// Scan disk for GPT and populate partition table
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

    // Write protective MBR
    let mut buf = [0u8; 512];
    disk.write_protective_mbr(&mut buf)
        .map_err(|_| GptError::IoError)?;

    // Create empty partition array
    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let block_size = BlockSize::BS_512;

    let mut entry_buf = [0u8; 16384];
    let entry_array = GptPartitionEntryArray::new(layout, block_size, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    // Update header CRC with empty partition array
    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    // Write both primary and secondary GPT
    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}

/// Find all free space regions on disk
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

    // Write both primary and secondary GPT
    write_gpt_both(&mut disk, &mut header, &entry_array)?;

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

    // Write both primary and secondary GPT
    write_gpt_both(&mut disk, &mut header, &entry_array)?;

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

    // Write both primary and secondary GPT
    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}
