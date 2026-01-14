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
        let title = "=== FORMAT PARTITION ===";
        screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);

        if self.partition_table.count() == 0 {
            let err = "ERROR: No partitions to format";
            screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        if self.selected_partition >= self.partition_table.count() {
            let err = "ERROR: Invalid partition selection";
            screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        let part = match self.partition_table.get(self.selected_partition) {
            Some(p) => p,
            None => {
                let err = "ERROR: Partition not found";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Show partition info - centered content area
        let content_width = 40;
        let content_x = screen.center_x(content_width);

        screen.put_str_at(content_x, 7, "Partition:", EFI_GREEN, EFI_BLACK);
        let mut buf = [0u8; 20];
        let len = Self::format_number(part.index as u64, &mut buf);
        screen.put_str_at(content_x + 11, 7, core::str::from_utf8(&buf[..len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(content_x, 8, "Type:", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(content_x + 11, 8, part.type_name(), EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(content_x, 9, "Size:", EFI_GREEN, EFI_BLACK);
        let len = Self::format_number(part.size_mb(), &mut buf);
        screen.put_str_at(content_x + 11, 9, core::str::from_utf8(&buf[..len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(content_x + 11 + len, 9, " MB", EFI_LIGHTGREEN, EFI_BLACK);

        let warn = "WARNING: This will erase all data on the partition!";
        screen.put_str_at(screen.center_x(warn.len()), 11, warn, EFI_LIGHTGREEN, EFI_BLACK);
        let confirm = "Format as FAT32?";
        screen.put_str_at(screen.center_x(confirm.len()), 13, confirm, EFI_GREEN, EFI_BLACK);
        let opts = "[Y] Yes    [N] No";
        screen.put_str_at(screen.center_x(opts.len()), 15, opts, EFI_DARKGREEN, EFI_BLACK);

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        screen.clear();
        let title2 = "=== FORMATTING PARTITION ===";
        screen.put_str_at(screen.center_x(title2.len()), 5, title2, EFI_LIGHTGREEN, EFI_BLACK);
        let formatting = "Formatting as FAT32...";
        screen.put_str_at(screen.center_x(formatting.len()), 7, formatting, EFI_GREEN, EFI_BLACK);

        // Get disk protocol
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                let err = "ERROR: Failed to access disk";
                screen.put_str_at(screen.center_x(err.len()), 9, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let mut adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                let err = "ERROR: Failed to create adapter";
                screen.put_str_at(screen.center_x(err.len()), 9, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Calculate partition size in sectors
        let partition_sectors = part.end_lba - part.start_lba + 1;

        // Format the partition
        let formatting = "Formatting as FAT32...";
        screen.put_str_at(screen.center_x(formatting.len()), 7, formatting, EFI_GREEN, EFI_BLACK);

        match morpheus_core::fs::format_fat32(&mut adapter, part.start_lba, partition_sectors) {
            Ok(()) => {
                let verifying = "Format complete. Verifying...";
                screen.put_str_at(screen.center_x(verifying.len()), 9, verifying, EFI_GREEN, EFI_BLACK);

                // Verify filesystem integrity
                match morpheus_core::fs::verify_fat32(&mut adapter, part.start_lba) {
                    Ok(()) => {
                        let success = "SUCCESS: Partition formatted as FAT32";
                        screen.put_str_at(screen.center_x(success.len()), 11, success, EFI_LIGHTGREEN, EFI_BLACK);
                        let verified = "Filesystem integrity verified";
                        screen.put_str_at(screen.center_x(verified.len()), 12, verified, EFI_LIGHTGREEN, EFI_BLACK);
                        let cont = "Press any key...";
                        screen.put_str_at(screen.center_x(cont.len()), 14, cont, EFI_DARKGREEN, EFI_BLACK);
                    }
                    Err(_) => {
                        let warn = "WARNING: Format succeeded but verification failed";
                        screen.put_str_at(screen.center_x(warn.len()), 11, warn, EFI_LIGHTGREEN, EFI_BLACK);
                        let corrupt = "Filesystem may be corrupted";
                        screen.put_str_at(screen.center_x(corrupt.len()), 12, corrupt, EFI_LIGHTGREEN, EFI_BLACK);
                        let cont = "Press any key...";
                        screen.put_str_at(screen.center_x(cont.len()), 14, cont, EFI_DARKGREEN, EFI_BLACK);
                    }
                }
            }
            Err(morpheus_core::fs::Fat32Error::PartitionTooSmall) => {
                let err = "ERROR: Partition too small (min 65MB)";
                screen.put_str_at(screen.center_x(err.len()), 9, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
            }
            Err(morpheus_core::fs::Fat32Error::PartitionTooLarge) => {
                let err = "ERROR: Partition too large (max 2TB)";
                screen.put_str_at(screen.center_x(err.len()), 9, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
            }
            Err(_) => {
                let err = "ERROR: Failed to format partition";
                screen.put_str_at(screen.center_x(err.len()), 9, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
            }
        }

        keyboard.wait_for_key();
    }
}
