use super::StorageManager;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;

impl StorageManager {
    pub(super) fn format_partition_ui(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        screen.clear();
        screen.put_str_at(5, 5, "=== FORMAT PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);

        if self.partition_table.count() == 0 {
            screen.put_str_at(
                5,
                7,
                "ERROR: No partitions to format",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        if self.selected_partition >= self.partition_table.count() {
            screen.put_str_at(
                5,
                7,
                "ERROR: Invalid partition selection",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        let part = match self.partition_table.get(self.selected_partition) {
            Some(p) => p,
            None => {
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Partition not found",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Show partition info
        screen.put_str_at(5, 7, "Partition:", EFI_GREEN, EFI_BLACK);
        let mut buf = [0u8; 20];
        let len = Self::format_number(part.index as u64, &mut buf);
        screen.put_str_at(
            16,
            7,
            core::str::from_utf8(&buf[..len]).unwrap_or("?"),
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        screen.put_str_at(5, 8, "Type:", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(16, 8, part.type_name(), EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(5, 9, "Size:", EFI_GREEN, EFI_BLACK);
        let len = Self::format_number(part.size_mb(), &mut buf);
        screen.put_str_at(
            16,
            9,
            core::str::from_utf8(&buf[..len]).unwrap_or("?"),
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(16 + len, 9, " MB", EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(
            5,
            11,
            "WARNING: This will erase all data on the partition!",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(5, 13, "Format as FAT32?", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(5, 15, "[Y] Yes    [N] No", EFI_DARKGREEN, EFI_BLACK);

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        screen.clear();
        screen.put_str_at(
            5,
            5,
            "=== FORMATTING PARTITION ===",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(5, 7, "Formatting as FAT32...", EFI_GREEN, EFI_BLACK);

        // Get disk protocol
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.put_str_at(
                    5,
                    9,
                    "ERROR: Failed to access disk",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let mut adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(
                    5,
                    9,
                    "ERROR: Failed to create adapter",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Calculate partition size in sectors
        let partition_sectors = part.end_lba - part.start_lba + 1;

        // Format the partition
        screen.put_str_at(5, 7, "Formatting as FAT32...", EFI_GREEN, EFI_BLACK);

        match morpheus_core::fs::format_fat32(&mut adapter, part.start_lba, partition_sectors) {
            Ok(()) => {
                screen.put_str_at(5, 9, "Format complete. Verifying...", EFI_GREEN, EFI_BLACK);

                // Verify filesystem integrity
                match morpheus_core::fs::verify_fat32(&mut adapter, part.start_lba) {
                    Ok(()) => {
                        screen.put_str_at(
                            5,
                            11,
                            "SUCCESS: Partition formatted as FAT32",
                            EFI_LIGHTGREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(
                            5,
                            12,
                            "Filesystem integrity verified",
                            EFI_LIGHTGREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(5, 14, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                    }
                    Err(_) => {
                        screen.put_str_at(
                            5,
                            11,
                            "WARNING: Format succeeded but verification failed",
                            EFI_LIGHTGREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(
                            5,
                            12,
                            "Filesystem may be corrupted",
                            EFI_LIGHTGREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(5, 14, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                    }
                }
            }
            Err(morpheus_core::fs::Fat32Error::PartitionTooSmall) => {
                screen.put_str_at(
                    5,
                    9,
                    "ERROR: Partition too small (min 65MB)",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            }
            Err(morpheus_core::fs::Fat32Error::PartitionTooLarge) => {
                screen.put_str_at(
                    5,
                    9,
                    "ERROR: Partition too large (max 2TB)",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            }
            Err(_) => {
                screen.put_str_at(
                    5,
                    9,
                    "ERROR: Failed to format partition",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            }
        }

        keyboard.wait_for_key();
    }
}
