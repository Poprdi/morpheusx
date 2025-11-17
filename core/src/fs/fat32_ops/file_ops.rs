// FAT32 file read/write operations

use super::context::Fat32Context;
use super::directory::add_dir_entry_to_cluster;
use super::super::Fat32Error;
use super::types::{DirEntry, ATTR_ARCHIVE, ATTR_DIRECTORY};
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

const SECTOR_SIZE: usize = 512;

pub fn write_file_in_directory<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    dir_cluster: u32,
    name: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    // Allocate clusters for file data
    let cluster_size = (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;
    let clusters_needed = ((data.len() + cluster_size - 1) / cluster_size).max(1);

    let mut file_clusters = Vec::new();
    for _ in 0..clusters_needed {
        let cluster = ctx.allocate_cluster(block_io, partition_start)?;
        file_clusters.push(cluster);
    }

    // Chain clusters together in FAT
    for i in 0..file_clusters.len() - 1 {
        ctx.write_fat_entry(
            block_io,
            partition_start,
            file_clusters[i],
            file_clusters[i + 1],
        )?;
    }
    // Last cluster is already marked with EOC by allocate_cluster

    // Write file data to clusters
    for (i, &cluster) in file_clusters.iter().enumerate() {
        let data_offset = i * cluster_size;
        let data_end = (data_offset + cluster_size).min(data.len());
        let chunk_size = data_end - data_offset;

        let mut cluster_data = vec![0u8; cluster_size];
        cluster_data[..chunk_size].copy_from_slice(&data[data_offset..data_end]);

        let sector = ctx.cluster_to_sector(cluster);
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
    }

    // Add directory entry
    add_dir_entry_to_cluster(
        block_io,
        partition_start,
        ctx,
        dir_cluster,
        name,
        file_clusters[0],
        data.len() as u32,
        ATTR_ARCHIVE,
    )?;

    Ok(())
}

pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    ctx: &Fat32Context,
    path: &str,
) -> Result<Vec<u8>, Fat32Error> {
    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        let sector = ctx.cluster_to_sector(current_cluster);
        let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

        let mut found = false;
        for sec_offset in 0..ctx.sectors_per_cluster {
            let mut sector_data = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(
                    Lba(partition_lba_start + sector as u64 + sec_offset as u64),
                    &mut sector_data,
                )
                .map_err(|_| Fat32Error::IoError)?;

            let entries = unsafe {
                core::slice::from_raw_parts(
                    sector_data.as_ptr() as *const DirEntry,
                    entries_per_sector,
                )
            };

            for entry in entries {
                if !entry.is_free() {
                    let mut test_entry = DirEntry::empty();
                    test_entry.set_name(part);

                    if entry.name == test_entry.name {
                        if is_last {
                            // Found the file - read its data
                            if entry.attr & ATTR_DIRECTORY != 0 {
                                return Err(Fat32Error::IoError); // Can't read directory as file
                            }

                            return read_file_data(
                                block_io,
                                partition_lba_start,
                                ctx,
                                entry.first_cluster(),
                                entry.file_size as usize,
                            );
                        } else {
                            current_cluster = entry.first_cluster();
                            found = true;
                            break;
                        }
                    }
                }
            }

            if found {
                break;
            }
        }

        if !found {
            return Err(Fat32Error::IoError);
        } // Path not found
    }

    Err(Fat32Error::IoError)
}

fn read_file_data<B: BlockIo>(
    block_io: &mut B,
    partition_start: u64,
    ctx: &Fat32Context,
    first_cluster: u32,
    file_size: usize,
) -> Result<Vec<u8>, Fat32Error> {
    let mut data = vec![0u8; file_size];
    let mut data_offset = 0;
    let cluster_size = (ctx.sectors_per_cluster * SECTOR_SIZE as u32) as usize;

    // Follow cluster chain
    let mut current_file_cluster = first_cluster;
    while current_file_cluster < 0x0FFFFFF8 {
        let sector = ctx.cluster_to_sector(current_file_cluster);
        let bytes_to_read = (file_size - data_offset).min(cluster_size);

        // Read cluster data
        let mut cluster_data = vec![0u8; cluster_size];
        for sec_offset in 0..ctx.sectors_per_cluster {
            let start = (sec_offset * SECTOR_SIZE as u32) as usize;
            let end = start + SECTOR_SIZE;
            block_io
                .read_blocks(
                    Lba(partition_start + sector as u64 + sec_offset as u64),
                    &mut cluster_data[start..end],
                )
                .map_err(|_| Fat32Error::IoError)?;
        }

        data[data_offset..data_offset + bytes_to_read]
            .copy_from_slice(&cluster_data[..bytes_to_read]);
        data_offset += bytes_to_read;

        if data_offset >= file_size {
            break;
        }

        // Get next cluster from FAT
        current_file_cluster = ctx.read_fat_entry(block_io, partition_start, current_file_cluster)?;
    }

    Ok(data)
}

pub fn file_exists<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    ctx: &Fat32Context,
    path: &str,
) -> Result<bool, Fat32Error> {
    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        let sector = ctx.cluster_to_sector(current_cluster);
        let entries_per_sector = SECTOR_SIZE / core::mem::size_of::<DirEntry>();

        let mut found = false;
        for sec_offset in 0..ctx.sectors_per_cluster {
            let mut sector_data = [0u8; SECTOR_SIZE];
            block_io
                .read_blocks(
                    Lba(partition_lba_start + sector as u64 + sec_offset as u64),
                    &mut sector_data,
                )
                .map_err(|_| Fat32Error::IoError)?;

            let entries = unsafe {
                core::slice::from_raw_parts(
                    sector_data.as_ptr() as *const DirEntry,
                    entries_per_sector,
                )
            };

            for entry in entries {
                if !entry.is_free() {
                    let mut test_entry = DirEntry::empty();
                    test_entry.set_name(part);

                    if entry.name == test_entry.name {
                        if is_last {
                            return Ok(entry.attr & ATTR_DIRECTORY == 0); // True if it's a file
                        } else {
                            current_cluster = entry.first_cluster();
                            found = true;
                            break;
                        }
                    }
                }
            }

            if found {
                break;
            }
        }

        if !found {
            return Ok(false);
        }
    }

    Ok(false)
}
