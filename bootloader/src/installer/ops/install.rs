// Bootloader self-installation module
// Handles installing Morpheus to EFI System Partition

use crate::BootServices;
extern crate alloc;

#[derive(Debug)]
pub enum InstallError {
    NoEsp,            // No EFI System Partition found
    EspTooSmall,      // ESP exists but not enough free space
    IoError,          // Disk I/O error
    ProtocolError,    // Failed to access UEFI protocols
    AlreadyInstalled, // Morpheus already installed
    NoFreeSpc,        // No free space to create ESP
    FormatFailed,     // Failed to format ESP
}

/// Information about located ESP
#[derive(Debug)]
pub struct EspInfo {
    pub disk_index: usize,
    pub partition_index: usize,
    pub start_lba: u64,
    pub size_mb: u64,
}

/// Find EFI System Partition on any disk
pub fn install_to_esp(
    bs: &BootServices,
    esp: &EspInfo,
    image_handle: *mut (),
) -> Result<(), InstallError> {
    unsafe {
        // Get loaded image protocol
        let loaded_image = crate::uefi::file_system::get_loaded_image(bs, image_handle)
            .map_err(|_| InstallError::ProtocolError)?;

        let image_base = (*loaded_image).image_base as *const u8;
        let image_size = (*loaded_image).image_size as usize;

        // Parse PE headers from running image
        use morpheus_persistent::pe::header::PeHeaders;

        let headers =
            PeHeaders::parse(image_base, image_size).map_err(|_| InstallError::ProtocolError)?;

        // Copy full memory image (needed for unrelocate - RVAs are memory-based)
        let mut binary_data = alloc::vec![0u8; image_size];
        core::ptr::copy_nonoverlapping(image_base, binary_data.as_mut_ptr(), image_size);

        // DEBUG: Check buffer IMMEDIATELY after copy, BEFORE unrelocate
        if binary_data.len() > 0x404 {
            let b0 = binary_data[0x400];
            let b1 = binary_data[0x401];
            let b2 = binary_data[0x402];
            let b3 = binary_data[0x403];
            if b0 == 0xAF && b1 == 0xAF && b2 == 0xAF && b3 == 0xAF {
                morpheus_core::logger::log("BUG: Memory at 0x400 is 0xAF BEFORE unrelocate!");
            } else {
                morpheus_core::logger::log("OK: Memory at 0x400 has code BEFORE unrelocate");
            }
        }

        // Unrelocate: reverse all DIR64 fixups + restore ImageBase
        let actual_load = image_base as u64;
        let delta_used = headers
            .unrelocate_image(&mut binary_data, actual_load)
            .map_err(|_| InstallError::ProtocolError)?;

        // Convert from RVA layout (memory) to file layout (disk)
        let file_layout_data = headers
            .rva_to_file_layout(&binary_data)
            .map_err(|_| InstallError::ProtocolError)?;

        // Use file-layout data for writing
        let binary_data = file_layout_data;

        // DEBUG: Log the delta being used
        if delta_used == 0 {
            morpheus_core::logger::log("ERROR: Delta is ZERO - heuristic failed!");
        } else if delta_used > 0 {
            morpheus_core::logger::log("Delta is positive (loaded higher than original)");
        } else {
            morpheus_core::logger::log("Delta is negative (loaded lower than original)");
        }

        // DEBUG: Check buffer AFTER unrelocate
        if binary_data.len() > 0x404 {
            let b0 = binary_data[0x400];
            let b1 = binary_data[0x401];
            let b2 = binary_data[0x402];
            let b3 = binary_data[0x403];
            if b0 == 0xAF && b1 == 0xAF && b2 == 0xAF && b3 == 0xAF {
                morpheus_core::logger::log("BUG: Buffer at 0x400 is 0xAF AFTER unrelocate!");
            } else {
                morpheus_core::logger::log("OK: Buffer at 0x400 has code AFTER unrelocate");
            }
        }

        // DON'T truncate! The image_size from LoadedImage includes ALL sections
        // including .morpheus which we need for self-replication!
        // The get_pe_file_size function would only see sections that UEFI loaded,
        // missing any metadata sections we injected post-build.
        //
        // The memory image is already the correct size from LoadedImage.

        // Pre-write verification: check buffer first 16 bytes at offset 0x400
        if binary_data.len() > 0x410 {
            let byte_1024 = binary_data[0x400];
            let byte_1025 = binary_data[0x401];
            let byte_1026 = binary_data[0x402];
            let byte_1027 = binary_data[0x403];

            if byte_1024 == 0xAF && byte_1025 == 0xAF && byte_1026 == 0xAF && byte_1027 == 0xAF {
                morpheus_core::logger::log(
                    "!!! Buffer already corrupted at 0x400 before FAT32 write !!!",
                );
            } else {
                morpheus_core::logger::log("Buffer OK at 0x400 - contains code, not 0xAF");
            }
        }

        // Get block IO for the disk containing the ESP
        let block_io = crate::uefi::disk::get_disk_protocol(bs, esp.disk_index)
            .map_err(|_| InstallError::ProtocolError)?;

        let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(&mut *block_io)
            .map_err(|_| InstallError::IoError)?;

        // Write directly to FAT32 partition
        // Bypasses UEFI FS protocol (works on runtime-created partitions)
        use morpheus_core::fs::fat32_ops;

        // DEBUG: Write buffer to /EFI/DEBUG.BIN before FAT32 write
        fat32_ops::write_file(&mut adapter, esp.start_lba, "/EFI/DEBUG.BIN", &binary_data)
            .map_err(|_| InstallError::IoError)?;

        // Write to fallback boot path - UEFI auto-detects and boots this
        fat32_ops::write_file(
            &mut adapter,
            esp.start_lba,
            "/EFI/BOOT/BOOTX64.EFI",
            &binary_data,
        )
        .map_err(|_| InstallError::IoError)?;

        // Verify write by reading back critical sectors
        #[cfg(feature = "fat32_debug")]
        {
            morpheus_core::logger::log("Verifying written file...");
            verify_written_file(&mut adapter, esp.start_lba, &binary_data)?;
        }

        Ok(())
    }
}

/// Check if Morpheus is already installed
