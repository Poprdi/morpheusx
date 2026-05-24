use super::{mb_to_lba, GptError};
use crate::disk::partition::PartitionType;
use gpt_disk_io::{BlockIo, Disk};
use gpt_disk_types::{
    guid, BlockSize, GptHeader, GptPartitionEntry, GptPartitionEntryArray, LbaLe, U32Le,
};

/// Writes primary and secondary headers + entry arrays. Both copies must
/// stay in sync or post-boot tools will flag a CRC mismatch.
fn write_gpt_both<B: BlockIo>(
    disk: &mut Disk<B>,
    header: &mut GptHeader,
    entry_array: &GptPartitionEntryArray,
) -> Result<(), GptError> {
    disk.write_primary_gpt_header(header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;
    disk.write_gpt_partition_entry_array(entry_array)
        .map_err(|_| GptError::IoError)?;

    // Secondary: swap my/alt LBA; entry array sits just before the alt header.
    let mut secondary_header = *header;
    let primary_lba = header.my_lba;
    let alternate_lba = header.alternate_lba;

    secondary_header.my_lba = alternate_lba;
    secondary_header.alternate_lba = primary_lba;

    let num_entries = header.number_of_partition_entries.to_u32() as u64;
    let entry_size = header.size_of_partition_entry.to_u32() as u64;
    let block_size = crate::fs::SECTOR_SIZE as u64;
    let entries_sectors = (num_entries * entry_size + block_size - 1) / block_size;
    let secondary_entry_lba = alternate_lba.to_u64() - entries_sectors;

    secondary_header.partition_entry_lba = LbaLe::from_u64(secondary_entry_lba);
    secondary_header.update_header_crc32();

    disk.write_secondary_gpt_header(&secondary_header, &mut [0u8; 512])
        .map_err(|_| GptError::IoError)?;

    let secondary_layout = secondary_header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let primary_storage = entry_array.storage();
    let mut secondary_buf = [0u8; 16384];
    secondary_buf[..primary_storage.len()].copy_from_slice(primary_storage);

    let secondary_entry_array =
        GptPartitionEntryArray::new(secondary_layout, BlockSize::BS_512, &mut secondary_buf)
            .map_err(|_| GptError::IoError)?;

    disk.write_gpt_partition_entry_array(&secondary_entry_array)
        .map_err(|_| GptError::IoError)?;

    disk.flush().map_err(|_| GptError::IoError)?;

    Ok(())
}

/// Initialize a fresh GPT: protective MBR, empty 128-entry array, dual headers.
pub fn create_gpt<B: BlockIo>(block_io: B, num_blocks: u64) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

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

    let mut buf = [0u8; 512];
    disk.write_protective_mbr(&mut buf)
        .map_err(|_| GptError::IoError)?;

    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let block_size = BlockSize::BS_512;

    let mut entry_buf = [0u8; 16384];
    let entry_array = GptPartitionEntryArray::new(layout, block_size, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}

pub fn create_partition<B: BlockIo>(
    block_io: B,
    partition_type: PartitionType,
    start_lba: u64,
    end_lba: u64,
) -> Result<(), GptError> {
    let mut disk = Disk::new(block_io).map_err(|_| GptError::IoError)?;

    let mut header = disk
        .read_primary_gpt_header(&mut [0u8; 512])
        .map_err(|_| GptError::InvalidHeader)?;

    let first_usable = header.first_usable_lba.to_u64();
    let last_usable = header.last_usable_lba.to_u64();

    if start_lba < first_usable || end_lba > last_usable || start_lba >= end_lba {
        return Err(GptError::InvalidSize);
    }

    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

    let mut entry_buf = [0u8; 16384];
    let mut entry_array = disk
        .read_gpt_partition_entry_array(layout, &mut entry_buf)
        .map_err(|_| GptError::IoError)?;

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

    let entry = entry_array
        .get_partition_entry_mut(slot.try_into().unwrap())
        .ok_or(GptError::NoSpace)?;

    entry.partition_type_guid = partition_type.to_gpt_guid();
    // TODO: real RNG for unique GUID.
    entry.unique_partition_guid = guid!("12345678-1234-5678-1234-567812345678");
    entry.starting_lba = LbaLe::from_u64(start_lba);
    entry.ending_lba = LbaLe::from_u64(end_lba);
    entry.attributes = Default::default();

    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}

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

    if let Some(entry) = entry_array.get_partition_entry_mut(partition_index.try_into().unwrap()) {
        *entry = GptPartitionEntry::default();
    } else {
        return Err(GptError::PartitionNotFound);
    }

    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}

/// `new_size_mb` must be smaller than current size; this is shrink-only.
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

    let entry = entry_array
        .get_partition_entry_mut(partition_index.try_into().unwrap())
        .ok_or(GptError::PartitionNotFound)?;

    if !entry.is_used() {
        return Err(GptError::PartitionNotFound);
    }

    let start_lba = entry.starting_lba.to_u64();
    let current_end_lba = entry.ending_lba.to_u64();
    let current_size_lba = current_end_lba - start_lba + 1;

    let new_size_lba = mb_to_lba(new_size_mb, crate::fs::SECTOR_SIZE as u32);

    if new_size_lba == 0 || new_size_lba >= current_size_lba {
        return Err(GptError::InvalidSize);
    }

    let new_end_lba = start_lba + new_size_lba - 1;
    entry.ending_lba = LbaLe::from_u64(new_end_lba);

    header.partition_entry_array_crc32 = entry_array.calculate_crc32();
    header.update_header_crc32();

    write_gpt_both(&mut disk, &mut header, &entry_array)?;

    Ok(())
}
