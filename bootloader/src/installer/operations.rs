pub fn find_esp(bs: &BootServices) -> Result<EspInfo, InstallError> {
    // Enumerate all disks
    let mut temp_disk_manager = morpheus_core::disk::manager::DiskManager::new();
    crate::uefi::disk::enumerate_disks(bs, &mut temp_disk_manager)
        .map_err(|_| InstallError::ProtocolError)?;

    let disk_count = temp_disk_manager.disk_count();

    // Scan each disk for ESP
    for disk_idx in 0..disk_count {
        // Get disk protocol
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, disk_idx) {
            Ok(ptr) => ptr,
            Err(_) => continue,
        };

        let block_io = unsafe { &mut *block_io_ptr };

        // Get block size before creating adapter to avoid borrow confict
        let media = unsafe { &*block_io.media };
        let block_size = media.block_size as usize;

        // Create adapter
        let adapter = match crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => continue,
        };

        // Try to read GPT
        let mut partition_table = morpheus_core::disk::partition::PartitionTable::new();
        if morpheus_core::disk::gpt_ops::scan_partitions(adapter, &mut partition_table, block_size)
            .is_err()
        {
            continue;
        }

        // Look for ESP partition
        for part_idx in 0..partition_table.count() {
            if let Some(part) = partition_table.get(part_idx) {
                if matches!(
                    part.partition_type,
                    morpheus_core::disk::partition::PartitionType::EfiSystem
                ) {
                    let size_mb = part.size_mb();

                    // Need at least 100MB for ESP
                    if size_mb < 100 {
                        continue;
                    }

                    return Ok(EspInfo {
                        disk_index: disk_idx,
                        partition_index: part_idx,
                        start_lba: part.start_lba,
                        size_mb,
                    });
                }
            }
        }
    }

    Err(InstallError::NoEsp)
}

#[cfg(feature = "fat32_debug")]
fn verify_written_file<B: gpt_disk_io::BlockIo>(
    adapter: &mut B,
    partition_start: u64,
    expected_data: &[u8],
) -> Result<(), InstallError> {
    use gpt_disk_types::Lba;
    use morpheus_core::fs::fat32_ops;

    // Read file back via FAT32
    let exists = fat32_ops::file_exists(adapter, partition_start, "/EFI/BOOT/BOOTX64.EFI")
        .map_err(|_| InstallError::IoError)?;

    if !exists {
        morpheus_core::logger::log("VERIFY FAIL: File does not exist after write");
        return Err(InstallError::IoError);
    }

    morpheus_core::logger::log("File exists - checking byte corruption...");

    // For simple check: read raw sectors where file should be and compare bytes
    // Offset 0x400 is where corruption starts (1024 bytes in, sector 2-3 region)
    // We'll check if bytes at offset 0x400-0x500 match expected or are 0xAF

    let check_offset = 0x400usize;
    let check_len = 256usize;

    if expected_data.len() > check_offset + check_len {
        let expected_slice = &expected_data[check_offset..check_offset + check_len];

        // Count how many bytes would be 0xAF if corrupted
        let mut af_count = 0u32;
        for &byte in expected_slice {
            if byte == 0xAF {
                af_count += 1;
            }
        }

        // Expected bytes at 0x400 should NOT all be 0xAF (code section)
        if af_count == check_len as u32 {
            morpheus_core::logger::log("WARN: Expected data at 0x400 is all 0xAF - can't verify");
        } else {
            morpheus_core::logger::log("Expected non-0xAF bytes at 0x400 - verification possible");
            // TODO: Actually read file back and compare
            // For now just log that we could verify if we had FAT32 read implemented
        }
    }

    Ok(())
}

/// Install bootloader to ESP using direct FAT32 write
/// Bypasses UEFI file system protocol (which fails on new partitions)
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
pub fn is_installed(bs: &BootServices) -> Result<bool, InstallError> {
    // Find ESP first
    let esp = find_esp(bs)?;

    unsafe {
        let block_io = crate::uefi::disk::get_disk_protocol(bs, esp.disk_index)
            .map_err(|_| InstallError::ProtocolError)?;

        let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(&mut *block_io)
            .map_err(|_| InstallError::IoError)?;

        use morpheus_core::fs::fat32_ops;

        let exists = fat32_ops::file_exists(&mut adapter, esp.start_lba, "/EFI/BOOT/BOOTX64.EFI")
            .map_err(|_| InstallError::IoError)?;

        Ok(exists)
    }
}

/// Create ESP and install bootloader in one operation
/// This finds free space, creates partition, formats FAT32, and installs
pub fn create_esp_and_install(
    bs: &BootServices,
    disk_index: usize,
) -> Result<EspInfo, InstallError> {
    // Get disk protocol
    let block_io_ptr = crate::uefi::disk::get_disk_protocol(bs, disk_index)
        .map_err(|_| InstallError::ProtocolError)?;

    let block_io = unsafe { &mut *block_io_ptr };

    // Get block size before creating adapter
    let media = unsafe { &*block_io.media };
    let block_size = media.block_size as usize;
    let total_blocks = media.last_block + 1;

    // Create adapter
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Read current partition table
    let mut partition_table = morpheus_core::disk::partition::PartitionTable::new();
    morpheus_core::disk::gpt_ops::scan_partitions(adapter, &mut partition_table, block_size)
        .map_err(|_| InstallError::IoError)?;

    // Recreate adapter for free space search
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Find free space (need at least 512MB for safety)
    let min_size_mb = 512;
    let min_sectors = (min_size_mb * 1024 * 1024) / block_size as u64;

    let free_regions = morpheus_core::disk::gpt_ops::find_free_space(adapter, block_size)
        .map_err(|_| InstallError::IoError)?;

    let region = free_regions
        .iter()
        .filter_map(|r| r.as_ref())
        .find(|r| (r.end_lba - r.start_lba + 1) >= min_sectors)
        .ok_or(InstallError::NoFreeSpc)?;

    // Recreate adapter for partition creation
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Calculate ESP size - use min_sectors but dont exceed free space
    let available_sectors = region.end_lba - region.start_lba + 1;
    let esp_sectors = min_sectors.min(available_sectors);
    let esp_end_lba = region.start_lba + esp_sectors - 1;

    // Create ESP partition
    let partition_type = morpheus_core::disk::partition::PartitionType::EfiSystem;
    morpheus_core::disk::gpt_ops::create_partition(
        adapter,
        partition_type,
        region.start_lba,
        esp_end_lba,
    )
    .map_err(|_| InstallError::IoError)?;

    // Recreate adapter for re-scan
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Re-scan to get partition index
    partition_table = morpheus_core::disk::partition::PartitionTable::new();
    morpheus_core::disk::gpt_ops::scan_partitions(adapter, &mut partition_table, block_size)
        .map_err(|_| InstallError::IoError)?;

    // Find the ESP we just created
    let mut esp_partition_index = None;
    for idx in 0..partition_table.count() {
        if let Some(part) = partition_table.get(idx) {
            if matches!(
                part.partition_type,
                morpheus_core::disk::partition::PartitionType::EfiSystem
            ) && part.start_lba == region.start_lba
            {
                esp_partition_index = Some(idx);
                break;
            }
        }
    }

    let partition_index = esp_partition_index.ok_or(InstallError::IoError)?;
    let part = partition_table
        .get(partition_index)
        .ok_or(InstallError::IoError)?;

    // Recreate adapter for formatting
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Format as FAT32
    let partition_sectors = part.end_lba - part.start_lba + 1;
    morpheus_core::fs::format_fat32(&mut adapter, part.start_lba, partition_sectors)
        .map_err(|_| InstallError::FormatFailed)?;

    // Recreate adapter for verification
    let mut adapter = crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io)
        .map_err(|_| InstallError::ProtocolError)?;

    // Verify filesystem
    morpheus_core::fs::verify_fat32(&mut adapter, part.start_lba)
        .map_err(|_| InstallError::FormatFailed)?;

    Ok(EspInfo {
        disk_index,
        partition_index,
        start_lba: part.start_lba,
        size_mb: part.size_mb(),
    })
}
