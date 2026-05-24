use super::super::Fat32Error;
use super::context::Fat32Context;
use super::types::{DirEntry, ATTR_DIRECTORY};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

const SECTOR_SIZE: usize = 512;

fn names_match_case_insensitive(a: &[u8; 11], b: &[u8; 11]) -> bool {
    for i in 0..11 {
        let ca = if a[i] >= b'a' && a[i] <= b'z' {
            a[i] - 32
        } else {
            a[i]
        };
        let cb = if b[i] >= b'a' && b[i] <= b'z' {
            b[i] - 32
        } else {
            b[i]
        };
        if ca != cb {
            return false;
        }
    }
    true
}

/// Match an LFN short-name alias like "ISO~1   " against base "ISO".
/// Handles the Windows/Linux long-filename short-name fallback.
fn entry_matches_lfn_short_name(entry_name: &[u8; 11], target: &[u8]) -> bool {
    if target.is_empty() {
        return false;
    }

    let mut base_end = 0;
    for i in 0..8 {
        if entry_name[i] == b'~' || entry_name[i] == b' ' {
            break;
        }
        base_end = i + 1;
    }

    if base_end == 0 {
        return false;
    }

    let entry_base = &entry_name[..base_end];
    let target_len = target.len().min(base_end);

    if target_len != base_end {
        return false;
    }

    for i in 0..target_len {
        let ce = if entry_base[i] >= b'a' && entry_base[i] <= b'z' {
            entry_base[i] - 32
        } else {
            entry_base[i]
        };
        let ct = if target[i] >= b'a' && target[i] <= b'z' {
            target[i] - 32
        } else {
            target[i]
        };
        if ce != ct {
            return false;
        }
    }

    if base_end < 8 && entry_name[base_end] == b'~' {
        return true;
    }

    false
}

pub fn ensure_directory_exists<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    parent_cluster: u32,
    name: &str,
) -> Result<u32, Fat32Error> {
    let sector = ctx.cluster_to_sector(parent_cluster);
    let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

    let mut test_entry = DirEntry::empty();
    test_entry.set_name(name);
    let target_name = test_entry.name;

    // For ".iso", LFN short name is "ISO~1   " — strip leading dot, match base.
    let name_upper = name.to_uppercase();
    let name_upper = name_upper.trim_start_matches('.');

    for sec_offset in 0..ctx.sectors_per_cluster {
        let mut sector_data = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &mut sector_data,
            )
            .map_err(|_| Fat32Error::IoError)?;

        let entries = unsafe {
            core::slice::from_raw_parts(sector_data.as_ptr() as *const DirEntry, entries_per_sector)
        };

        for entry in entries {
            if !entry.is_free() && entry.attr & ATTR_DIRECTORY != 0 {
                let entry_name = entry.name;

                if names_match_case_insensitive(&entry_name, &target_name) {
                    return Ok(entry.first_cluster());
                }
                if entry_matches_lfn_short_name(&entry_name, name_upper.as_bytes()) {
                    return Ok(entry.first_cluster());
                }
            }
        }
    }

    create_directory_in_parent(block_io, partition_start, ctx, parent_cluster, name)
}

pub fn create_directory_in_parent<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    parent_cluster: u32,
    name: &str,
) -> Result<u32, Fat32Error> {
    let new_cluster = ctx.allocate_cluster(block_io, partition_start)?;

    let mut dot_entry = DirEntry::empty();
    dot_entry.name = *b".          ";
    dot_entry.attr = ATTR_DIRECTORY;
    dot_entry.set_first_cluster(new_cluster);

    let mut dotdot_entry = DirEntry::empty();
    dotdot_entry.name = *b"..         ";
    dotdot_entry.attr = ATTR_DIRECTORY;
    dotdot_entry.set_first_cluster(parent_cluster);

    let mut sector_data = [0u8; SECTOR_SIZE];
    let entries =
        unsafe { core::slice::from_raw_parts_mut(sector_data.as_mut_ptr() as *mut DirEntry, 2) };
    entries[0] = dot_entry;
    entries[1] = dotdot_entry;

    let sector = ctx.cluster_to_sector(new_cluster);

    block_io
        .write_blocks(Lba(partition_start + sector as u64), &sector_data)
        .map_err(|_| Fat32Error::IoError)?;

    let zero_sector = [0u8; SECTOR_SIZE];
    for sec_offset in 1..ctx.sectors_per_cluster {
        block_io
            .write_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &zero_sector,
            )
            .map_err(|_| Fat32Error::IoError)?;
    }

    add_dir_entry_to_cluster(
        block_io,
        partition_start,
        ctx,
        parent_cluster,
        name,
        new_cluster,
        0,
        ATTR_DIRECTORY,
    )?;

    Ok(new_cluster)
}

#[allow(clippy::too_many_arguments)]
pub fn add_dir_entry_to_cluster<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    cluster: u32,
    name: &str,
    first_cluster: u32,
    file_size: u32,
    attr: u8,
) -> Result<(), Fat32Error> {
    let sector = ctx.cluster_to_sector(cluster);
    let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

    for sec_offset in 0..ctx.sectors_per_cluster {
        let mut sector_data = [0u8; SECTOR_SIZE];
        block_io
            .read_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &mut sector_data,
            )
            .map_err(|_| Fat32Error::IoError)?;

        let entries = unsafe {
            core::slice::from_raw_parts_mut(
                sector_data.as_mut_ptr() as *mut DirEntry,
                entries_per_sector,
            )
        };

        for entry in entries.iter_mut() {
            if entry.is_free() {
                entry.set_name(name);
                entry.attr = attr;
                entry.set_first_cluster(first_cluster);
                entry.file_size = file_size;

                block_io
                    .write_blocks(
                        Lba(partition_start + sector as u64 + sec_offset as u64),
                        &sector_data,
                    )
                    .map_err(|_| Fat32Error::IoError)?;

                return Ok(());
            }
        }
    }

    Err(Fat32Error::IoError) // dir full
}

pub fn create_directory<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    ctx: &Fat32Context,
    path: &str,
) -> Result<(), Fat32Error> {
    let path = path.trim_start_matches('/');

    const MAX_PATH_PARTS: usize = 8;
    let mut parts: [&str; MAX_PATH_PARTS] = [""; MAX_PATH_PARTS];
    let mut parts_count = 0;
    for part in path.split('/') {
        if parts_count >= MAX_PATH_PARTS {
            return Err(Fat32Error::IoError);
        }
        parts[parts_count] = part;
        parts_count += 1;
    }

    let mut current_cluster = ctx.root_cluster;
    for i in 0..parts_count {
        current_cluster = ensure_directory_exists(
            block_io,
            partition_lba_start,
            ctx,
            current_cluster,
            parts[i],
        )?;
    }

    Ok(())
}
