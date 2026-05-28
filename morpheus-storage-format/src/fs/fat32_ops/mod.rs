//! Minimal FAT32 ops for bootloader install.

mod context;
mod directory;
mod file_ops;
pub mod filename;
mod types;

use super::Fat32Error;
use crate::uefi_alloc;
use context::Fat32Context;
use gpt_disk_io::BlockIo;

extern crate alloc;
use alloc::vec::Vec;

/// (bytes_written, total_bytes, message)
pub type ProgressCallback<'a> = Option<&'a mut dyn FnMut(usize, usize, &str)>;

pub fn write_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    write_file_with_progress(block_io, partition_lba_start, path, data, &mut None)
}

pub fn write_file_with_progress<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
    progress: &mut ProgressCallback,
) -> Result<(), Fat32Error> {
    write_file_with_progress_uefi(
        block_io,
        partition_lba_start,
        path,
        data,
        progress,
        None,
        None,
    )
}

/// Pass Some(boot_services_*) to use UEFI allocate_pages (pre-EBS).
pub fn write_file_with_progress_uefi<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
    progress: &mut ProgressCallback,
    boot_services_alloc: Option<uefi_alloc::AllocatePages>,
    boot_services_free: Option<uefi_alloc::FreePages>,
) -> Result<(), Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;

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
        let part = parts[i];
        let is_last = i == parts_count - 1;

        if !is_last {
            current_cluster = directory::ensure_directory_exists(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
            )?;
        } else {
            file_ops::write_file_in_directory_with_progress_uefi(
                block_io,
                partition_lba_start,
                &ctx,
                current_cluster,
                part,
                data,
                progress,
                boot_services_alloc,
                boot_services_free,
            )?;
        }
    }

    block_io.flush().map_err(|_| Fat32Error::IoError)?;
    Ok(())
}

/// Creates the full path; intermediate dirs implicit.
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

pub fn read_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<Vec<u8>, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;
    file_ops::read_file(block_io, partition_lba_start, &ctx, path)
}

pub fn file_exists<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
) -> Result<bool, Fat32Error> {
    let ctx = Fat32Context::from_boot_sector(block_io, partition_lba_start)?;
    file_ops::file_exists(block_io, partition_lba_start, &ctx, path)
}
