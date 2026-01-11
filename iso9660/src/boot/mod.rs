//! El Torito boot support
//!
//! Parsing boot catalogs and boot images from ISO9660 volumes.

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

/// Find boot image from El Torito boot catalog
///
/// Extracts the bootable image entry from an ISO's El Torito catalog.
/// Useful for booting live ISOs (Tails, Ubuntu, etc.) or custom kernels.
///
/// # Arguments
/// * `block_io` - Block device
/// * `volume` - Mounted volume
///
/// # Returns
/// Boot image entry with LBA, size, platform, and media type
///
/// # Example
/// ```ignore
/// use iso9660::{mount, find_boot_image};
///
/// let volume = mount(&mut block_io, 0)?;
/// let boot = find_boot_image(&mut block_io, &volume)?;
/// println!("Boot image at sector {}, {} bytes",
///     boot.load_rba,
///     boot.sector_count * 512
/// );
/// ```
pub fn find_boot_image<B: BlockIo>(block_io: &mut B, volume: &VolumeInfo) -> Result<BootImage> {
    // Check if boot catalog exists
    let catalog_lba = volume.boot_catalog_lba.ok_or(Iso9660Error::NoBootCatalog)?;

    // Read boot catalog sector
    let mut buffer = [0u8; SECTOR_SIZE];
    block_io
        .read_blocks(Lba(catalog_lba as u64), &mut buffer)
        .map_err(|_| Iso9660Error::IoError)?;

    // Parse validation entry (first 32 bytes)
    let validation = unsafe { &*(buffer.as_ptr() as *const ValidationEntry) };

    if !validation.is_valid() {
        return Err(Iso9660Error::InvalidBootCatalog);
    }

    // Parse initial/default entry (next 32 bytes)
    let initial = unsafe { &*(buffer[32..].as_ptr() as *const BootEntry) };

    if !initial.is_bootable() {
        return Err(Iso9660Error::InvalidBootEntry);
    }

    // Build BootImage from entry
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
