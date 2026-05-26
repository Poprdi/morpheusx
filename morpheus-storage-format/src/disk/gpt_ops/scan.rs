use super::GptError;
use crate::disk::partition::{PartitionInfo, PartitionTable, PartitionType};
use gpt_disk_io::{BlockIo, Disk};

/// Missing or unreadable GPT is reported via `has_gpt = false`, not an error.
pub fn scan_partitions<B: BlockIo>(
    block_io: B,
    partition_table: &mut PartitionTable,
    block_size_bytes: usize,
) -> Result<(), GptError> {
    partition_table.clear();

    let mut disk = match Disk::new(block_io) {
        Ok(d) => d,
        Err(_) => {
            partition_table.has_gpt = false;
            return Ok(());
        },
    };

    let header = match disk.read_primary_gpt_header(&mut [0u8; 512]) {
        Ok(h) => h,
        Err(_) => {
            partition_table.has_gpt = false;
            return Ok(());
        },
    };

    partition_table.has_gpt = true;

    let layout = match header.get_partition_entry_array_layout() {
        Ok(l) => l,
        Err(_) => {
            partition_table.has_gpt = false;
            return Ok(());
        },
    };

    let mut entry_buf = [0u8; 4096];
    let entry_buffer = &mut entry_buf[..block_size_bytes];

    let iter = match disk.gpt_partition_entry_array_iter(layout, entry_buffer) {
        Ok(it) => it,
        Err(_) => {
            partition_table.has_gpt = false;
            return Ok(());
        },
    };

    for (index, entry_result) in iter.enumerate() {
        let entry = entry_result.map_err(|_| GptError::IoError)?;

        if !entry.is_used() {
            continue;
        }

        // Copy out: packed struct, can't take an unaligned reference.
        let guid = entry.partition_type_guid;
        let partition_type = PartitionType::from_gpt_guid(&guid);

        let info = PartitionInfo {
            index: index as u32,
            partition_type,
            start_lba: entry.starting_lba.to_u64(),
            end_lba: entry.ending_lba.to_u64(),
        };

        if partition_table.add_partition(info).is_err() {
            break;
        }
    }

    Ok(())
}
