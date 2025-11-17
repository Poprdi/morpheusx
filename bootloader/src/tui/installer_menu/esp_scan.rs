// ESP scanning operations

use crate::installer::EspInfo;
use crate::BootServices;
use alloc::vec::Vec;

pub fn scan_for_esps(bs: &BootServices) -> Vec<EspInfo> {
    let mut esp_list = Vec::new();

    // Scan all disks for ESPs
    let mut temp_disk_manager = morpheus_core::disk::manager::DiskManager::new();
    if crate::uefi::disk::enumerate_disks(bs, &mut temp_disk_manager).is_err() {
        return esp_list;
    }

    let disk_count = temp_disk_manager.disk_count();

    for disk_idx in 0..disk_count {
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, disk_idx) {
            Ok(ptr) => ptr,
            Err(_) => continue,
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let media = unsafe { &*block_io.media };
        let block_size = media.block_size as usize;

        let adapter = match crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => continue,
        };

        let mut partition_table = morpheus_core::disk::partition::PartitionTable::new();
        if morpheus_core::disk::gpt_ops::scan_partitions(adapter, &mut partition_table, block_size)
            .is_err()
        {
            continue;
        }

        // Find all ESP partitions on this disk
        for part_idx in 0..partition_table.count() {
            if let Some(part) = partition_table.get(part_idx) {
                if matches!(
                    part.partition_type,
                    morpheus_core::disk::partition::PartitionType::EfiSystem
                ) {
                    esp_list.push(EspInfo {
                        disk_index: disk_idx,
                        partition_index: part_idx,
                        start_lba: part.start_lba,
                        size_mb: part.size_mb(),
                    });
                }
            }
        }
    }

    esp_list
}
