// FAT32 filesystem operations - minimal implementation for bootloader installation

mod context;
mod directory;
mod file_ops;
mod types;

use super::Fat32Error;
use context::Fat32Context;
use gpt_disk_io::BlockIo;

extern crate alloc;
use alloc::vec::Vec;

/// Write file to FAT32 partition
pub fn write_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

    // Parse path
    let path = path.trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();

    // Navigate/create directory structure
    let mut current_cluster = ctx.root_cluster;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;

        if !is_last {
            // This is a directory component
            current_cluster = directory::ensure_directory_exists(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
            )?;
        } else {
            // This is the file name - create/write it
            file_ops::write_file_in_directory(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
                data,
            )?;
        }
    }

    block_io.flush().map_err(|_| Fat32Error::IoError)?;
    Ok(())
}

/// Create directory (creates full path)
pub fn create_directory<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<(), Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;
    directory::create_directory(block_io, partition_lba_start, &ctx, path)?;
    block_io.flush().map_err(|_| Fat32Error::IoError)?;
    Ok(())
}

/// Read file data from FAT32 partition
pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<Vec<u8>, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;
    file_ops::read_file(block_io, partition_lba_start, &ctx, path)
}

/// Check if file exists
pub fn file_exists<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<bool, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;
    file_ops::file_exists(block_io, partition_lba_start, &ctx, path)
}
