// FAT32 directory operations

use super::context::Fat32Context;
use super::super::Fat32Error;
use super::types::{DirEntry, ATTR_DIRECTORY};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

extern crate alloc;
use alloc::vec;

const SECTOR_SIZE: usize = 512;

pub fn ensure_directory_exists<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    parent_cluster: u32,
    name: &str,
) -> Result<u32, Fat32Error> {
    // Read parent directory
    let sector = ctx.cluster_to_sector(parent_cluster);
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
            core::slice::from_raw_parts(sector_data.as_ptr() as *const DirEntry, entries_per_sector)
        };

        // Check if directory already exists
        for entry in entries {
            if !entry.is_free() && entry.attr & ATTR_DIRECTORY != 0 {
                let mut entry_name = [0u8; 11];
                entry_name.copy_from_slice(&entry.name);

                let mut test_entry = DirEntry::empty();
                test_entry.set_name(name);

                if entry_name == test_entry.name {
                    return Ok(entry.first_cluster());
                }
            }
        }
    }

    // Directory doesn't exist - create it
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

    // Initialize new directory cluster with . and .. entries
    let cluster_size = (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;
    let mut cluster_data = vec![0u8; cluster_size];

    // Create '.' entry (points to self)
    let mut dot_entry = DirEntry::empty();
    dot_entry.name = *b".          "; // '.' padded with spaces
    dot_entry.attr = ATTR_DIRECTORY;
    dot_entry.set_first_cluster(new_cluster);

    // Create '..' entry (points to parent)
    let mut dotdot_entry = DirEntry::empty();
    dotdot_entry.name = *b"..         "; // '..' padded with spaces
    dotdot_entry.attr = ATTR_DIRECTORY;
    dotdot_entry.set_first_cluster(parent_cluster);

    // Write entries to cluster data
    let entries =
        unsafe { core::slice::from_raw_parts_mut(cluster_data.as_mut_ptr() as *mut DirEntry, 2) };
    entries[0] = dot_entry;
    entries[1] = dotdot_entry;

    let sector = ctx.cluster_to_sector(new_cluster);
    for sec_offset in 0..ctx.sectors_per_cluster {
        let start = (sec_offset * SECTOR_SIZE as u32) as usize;
        let end = start + SECTOR_SIZE;
        block_io
            .write_blocks(
                Lba(partition_start + sector as u64 + sec_offset as u64),
                &cluster_data[start..end],
            )
            .map_err(|_| Fat32Error::IoError)?;
    }

    // Add entry to parent directory
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

        // Find first free entry
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

    Err(Fat32Error::IoError) // Directory full
}

pub fn create_directory<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    ctx: &Fat32Context,
    path: &str,
) -> Result<(), Fat32Error> {
    let path = path.trim_start_matches('/');
    let parts: alloc::vec::Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for part in parts {
        current_cluster =
            ensure_directory_exists(block_io, partition_lba_start, ctx, current_cluster, part)?;
    }

    Ok(())
}
