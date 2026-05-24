//! El Torito boot catalog and boot image extraction.

pub mod catalog;
pub mod entry;
pub mod platform;
pub mod validation;

use crate::error::{Iso9660Error, Result};
use crate::types::{BootImage, BootPlatform, VolumeInfo, SECTOR_SIZE};
use entry::BootEntry;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;
use validation::ValidationEntry;

/// Extract the initial/default boot image from a mounted volume's El Torito catalog.
pub fn find_boot_image<B: BlockIo>(block_io: &mut B, volume: &VolumeInfo) -> Result<BootImage> {
    let catalog_lba = volume.boot_catalog_lba.ok_or(Iso9660Error::NoBootCatalog)?;

    let mut buffer = [0u8; SECTOR_SIZE];
    block_io
        .read_blocks(Lba(catalog_lba as u64), &mut buffer)
        .map_err(|_| Iso9660Error::IoError)?;

    // SAFETY: ValidationEntry and BootEntry are repr(C, packed) 32-byte structs;
    // SECTOR_SIZE (2048) covers both offsets.
    let validation = unsafe { &*(buffer.as_ptr() as *const ValidationEntry) };

    if !validation.is_valid() {
        return Err(Iso9660Error::InvalidBootCatalog);
    }

    let initial = unsafe { &*(buffer[32..].as_ptr() as *const BootEntry) };

    if !initial.is_bootable() {
        return Err(Iso9660Error::InvalidBootEntry);
    }

    Ok(BootImage {
        bootable: true,
        media_type: initial.media_type(),
        load_segment: initial.load_segment,
        system_type: initial.system_type,
        sector_count: initial.sector_count,
        load_rba: initial.load_rba,
        platform: BootPlatform::from_id(validation.platform_id),
    })
}
