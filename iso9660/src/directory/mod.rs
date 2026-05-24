//! Directory record parsing and path lookup.

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
#[allow(dead_code)]
extern "C" {
    fn morpheus_log(msg: *const u8, len: usize);
}

#[cfg(feature = "trace")]
#[allow(dead_code)]
fn trace(msg: &str) {
    unsafe { morpheus_log(msg.as_ptr(), msg.len()) };
}

#[cfg(not(feature = "trace"))]
#[allow(dead_code)]
fn trace(_msg: &str) {}

/// Resolve a `/`-separated, case-insensitive path to a `FileEntry`.
/// An empty path or "/" returns a synthesized root entry.
pub fn find_file<B: BlockIo>(
    block_io: &mut B,
    volume: &VolumeInfo,
    path: &str,
) -> Result<FileEntry> {
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();

    if components.len() > MAX_DIRECTORY_DEPTH {
        return Err(Iso9660Error::PathTooLong);
    }

    let mut current_lba = volume.root_extent_lba;
    let mut current_len = volume.root_extent_len;

    for (depth, component) in components.iter().enumerate() {
        let is_last = depth == components.len() - 1;

        let iter = iterator::DirectoryIterator::new(block_io, current_lba, current_len);

        let mut found = None;
        let mut _entry_count = 0u32;
        for result in iter {
            let entry = result?;
            _entry_count += 1;
            if entry.name.eq_ignore_ascii_case(component) {
                found = Some(entry);
                break;
            }
        }

        match found {
            Some(entry) => {
                if is_last {
                    return Ok(entry);
                }
                if !entry.flags.directory {
                    return Err(Iso9660Error::NotFound);
                }
                current_lba = entry.extent_lba;
                current_len = entry.data_length;
            }
            None => return Err(Iso9660Error::NotFound),
        }
    }

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
