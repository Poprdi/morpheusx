use super::{FreeRegion, GptError};
use gpt_disk_io::{BlockIo, Disk};

/// Returns up to 16 gaps between used partitions, sorted by start LBA.
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

    let layout = header
        .get_partition_entry_array_layout()
        .map_err(|_| GptError::InvalidHeader)?;

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

    // O(n^2) sort; n<=16.
    for i in 0..used_count {
        for j in i + 1..used_count {
            if used_ranges[j].0 < used_ranges[i].0 {
                used_ranges.swap(i, j);
            }
        }
    }

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

    if current <= last_usable && region_count < 16 {
        regions[region_count] = Some(FreeRegion {
            start_lba: current,
            end_lba: last_usable,
        });
    }

    Ok(regions)
}
