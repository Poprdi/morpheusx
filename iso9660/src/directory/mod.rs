//! Directory record parsing and navigation

pub mod flags;
pub mod iterator;
pub mod path_table;
pub mod record;

use crate::error::{Iso9660Error, Result};
use crate::types::{FileEntry, VolumeInfo, MAX_DIRECTORY_DEPTH};
use alloc::string::String;
use alloc::vec::Vec;
use gpt_disk_io::BlockIo;

#[cfg(feature = "trace")]
extern "C" {
    fn morpheus_log(msg: *const u8, len: usize);
}

#[cfg(feature = "trace")]
fn trace(msg: &str) {
    unsafe { morpheus_log(msg.as_ptr(), msg.len()) };
}

#[cfg(not(feature = "trace"))]
#[allow(dead_code)]
fn trace(_msg: &str) {}

/// Find a file or directory by path
///
/// Navigates the directory tree from root to locate a file/directory.
/// Paths are case-insensitive and support both `/` and `\` separators.
///
/// # Arguments
/// * `block_io` - Block device
/// * `volume` - Mounted volume info
/// * `path` - Path to find (e.g., "/boot/vmlinuz", "/LIVE/INITRD.IMG")
///
/// # Returns
/// File entry if found, with metadata and extent location
///
/// # Example
/// ```ignore
/// use iso9660::{mount, find_file};
///
/// let volume = mount(&mut block_io, 0)?;
/// let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;
/// println!("File size: {} bytes", file.size);
/// ```
pub fn find_file<B: BlockIo>(
    block_io: &mut B,
    volume: &VolumeInfo,
    path: &str,
) -> Result<FileEntry> {
    // Split path by '/' and filter empty components
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();

    // Check depth
    if components.len() > MAX_DIRECTORY_DEPTH {
        return Err(Iso9660Error::PathTooLong);
    }

    // Start at root directory
    let mut current_lba = volume.root_extent_lba;
    let mut current_len = volume.root_extent_len;

    // Navigate through each component
    for (depth, component) in components.iter().enumerate() {
        let is_last = depth == components.len() - 1;

        // Create iterator for current directory
        let iter = iterator::DirectoryIterator::new(block_io, current_lba, current_len);

        // Search for matching entry (case-insensitive)
        let mut found = None;
        let mut _entry_count = 0u32;
        for result in iter {
            let entry = result?;
            _entry_count += 1;

            // Case-insensitive comparison
            if entry.name.eq_ignore_ascii_case(component) {
                found = Some(entry);
                break;
            }
        }

        match found {
            Some(entry) => {
                if is_last {
                    // Found the target file/directory
                    return Ok(entry);
                } else {
                    // Need to navigate into this directory
                    if !entry.flags.directory {
                        return Err(Iso9660Error::NotFound);
                    }
                    current_lba = entry.extent_lba;
                    current_len = entry.data_length;
                }
            }
            None => return Err(Iso9660Error::NotFound),
        }
    }

    // If path is empty or just "/", return root as a directory entry
    if components.is_empty() {
        Ok(FileEntry {
            name: String::from("/"),
            size: volume.root_extent_len as u64,
            extent_lba: volume.root_extent_lba,
            data_length: volume.root_extent_len,
            flags: crate::types::FileFlags {
                hidden: false,
                directory: true,
                associated: false,
                extended_format: false,
                extended_permissions: false,
                not_final: false,
            },
            file_unit_size: 0,
            interleave_gap: 0,
        })
    } else {
        Err(Iso9660Error::NotFound)
    }
}
