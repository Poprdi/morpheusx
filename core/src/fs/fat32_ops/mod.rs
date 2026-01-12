// FAT32 filesystem operations - minimal implementation for bootloader installation

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
use alloc::vec::Vec; // Only used by read_file (post-EBS)

/// Progress callback type: (bytes_written, total_bytes, message)
pub type ProgressCallback<'a> = Option<&'a mut dyn FnMut(usize, usize, &str)>;

/// Write file to FAT32 partition
pub fn write_file<B: BlockIo>(
    block_io: &mut B,
    partition_lba_start: u64,
    path: &str,
    data: &[u8],
) -> Result<(), Fat32Error> {
    write_file_with_progress(block_io, partition_lba_start, path, data, &mut None)
}

/// Write file to FAT32 partition with progress reporting
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

/// Write file to FAT32 partition with progress reporting and UEFI allocation support
/// Pass Some() for boot_services_* to use UEFI allocate_pages for pre-EBS memory
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

    // Parse path - use fixed array instead of Vec (no heap allocation pre-EBS)
    // Max 8 path components should be plenty for EFI paths
    let path = path.trim_start_matches('/');
    const MAX_PATH_PARTS: usize = 8;
    let mut parts: [&str; MAX_PATH_PARTS] = [""; MAX_PATH_PARTS];
    let mut parts_count = 0;
    for part in path.split('/') {
        if parts_count >= MAX_PATH_PARTS {
            return Err(Fat32Error::IoError); // Path too deep
        }
        parts[parts_count] = part;
        parts_count += 1;
    }

    // Navigate/create directory structure
    let mut current_cluster = ctx.root_cluster;
    for i in 0..parts_count {
        let part = parts[i];
        let is_last = i == parts_count - 1;

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
            // This is the file name - create/write it with UEFI support
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
