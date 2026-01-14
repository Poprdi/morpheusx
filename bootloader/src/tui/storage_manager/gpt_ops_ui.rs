use super::StorageManager;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::gpt_ops;

impl StorageManager {
    pub(super) fn scan_disk(&mut self, disk_index: usize, bs: &BootServices) -> Result<(), usize> {
        let block_io_ptr = crate::uefi::disk::get_disk_protocol(bs, disk_index)?;
        let block_io = unsafe { &mut *block_io_ptr };

        let media = unsafe { &*block_io.media };
        let block_size = media.block_size as usize;

        let adapter = UefiBlockIoAdapter::new(block_io).map_err(|_| 1usize)?;

        // Use core module to scan partitions
        gpt_ops::scan_partitions(adapter, &mut self.partition_table, block_size).map_err(|e| {
            match e {
                gpt_ops::GptError::IoError => 2usize,
                gpt_ops::GptError::InvalidHeader => 3usize,
                gpt_ops::GptError::NoSpace => 4usize,
                gpt_ops::GptError::PartitionNotFound => 5usize,
                gpt_ops::GptError::OverlappingPartitions => 6usize,
                gpt_ops::GptError::InvalidSize => 7usize,
                gpt_ops::GptError::AlignmentError => 8usize,
            }
        })?;

        // Calculate free space if GPT is present
        if self.partition_table.has_gpt {
            let block_io = unsafe { &mut *block_io_ptr };
            let adapter = UefiBlockIoAdapter::new(block_io).map_err(|_| 1usize)?;
            self.free_space_mb =
                gpt_ops::calculate_total_free_space(adapter, block_size).unwrap_or(0);
        } else {
            self.free_space_mb = 0;
        }

        Ok(())
    }

    pub(super) fn create_gpt_interactive(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        screen.clear();
        let title = "=== CREATE GPT PARTITION TABLE ===";
        screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);
        let warn = "WARNING: This will erase all data on the disk!";
        screen.put_str_at(screen.center_x(warn.len()), 7, warn, EFI_LIGHTGREEN, EFI_BLACK);
        let confirm = "Press Y to confirm, any other key to cancel";
        screen.put_str_at(screen.center_x(confirm.len()), 9, confirm, EFI_GREEN, EFI_BLACK);

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            screen.clear();
            let cancelled = "GPT creation cancelled";
            screen.put_str_at(screen.center_x(cancelled.len()), 5, cancelled, EFI_GREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 7, cont, EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        screen.clear();
        let creating = "Creating GPT table...";
        screen.put_str_at(screen.center_x(creating.len()), 5, creating, EFI_LIGHTGREEN, EFI_BLACK);

        // Get the block IO protocol for current disk
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                let err = "ERROR: Failed to get BlockIO protocol";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let media = unsafe { &*block_io.media };
        let disk_size_lba = media.last_block + 1;

        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                let err = "ERROR: Failed to create adapter";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Use core module to create GPT
        match gpt_ops::create_gpt(adapter, disk_size_lba) {
            Ok(()) => {
                self.partition_table.clear();
                self.partition_table.has_gpt = true;

                let success = "GPT table created successfully!";
                screen.put_str_at(screen.center_x(success.len()), 7, success, EFI_GREEN, EFI_BLACK);
                let cont = "Press any key to continue...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error writing to disk",
                    gpt_ops::GptError::InvalidHeader => "Invalid header generated",
                    _ => "Unknown error",
                };
                let err = "ERROR: Failed to create GPT";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(screen.center_x(err_msg.len()), 9, err_msg, EFI_GREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
}
