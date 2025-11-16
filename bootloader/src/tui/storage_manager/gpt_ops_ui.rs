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
        screen.put_str_at(
            5,
            5,
            "=== CREATE GPT PARTITION TABLE ===",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            5,
            7,
            "WARNING: This will erase all data on the disk!",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            5,
            9,
            "Press Y to confirm, any other key to cancel",
            EFI_GREEN,
            EFI_BLACK,
        );

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            screen.clear();
            screen.put_str_at(5, 5, "GPT creation cancelled", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(5, 7, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        screen.clear();
        screen.put_str_at(5, 5, "Creating GPT table...", EFI_LIGHTGREEN, EFI_BLACK);

        // Get the block IO protocol for current disk
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to get BlockIO protocol",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
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
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to create adapter",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        // Use core module to create GPT
        match gpt_ops::create_gpt(adapter, disk_size_lba) {
            Ok(()) => {
                self.partition_table.clear();
                self.partition_table.has_gpt = true;

                screen.put_str_at(
                    5,
                    7,
                    "GPT table created successfully!",
                    EFI_GREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(
                    5,
                    9,
                    "Press any key to continue...",
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error writing to disk",
                    gpt_ops::GptError::InvalidHeader => "Invalid header generated",
                    _ => "Unknown error",
                };
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to create GPT",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 9, err_msg, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
}
