use super::super::StorageManager;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::tui::widgets::textbox::TextBox;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::gpt_ops;

impl StorageManager {
    pub(in super::super) fn delete_partition_ui(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        if self.partition_table.count() == 0 {
            screen.clear();
            screen.put_str_at(5, 5, "No partitions to delete", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(5, 7, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        // Get selected partition info
        let partition = match self.partition_table.get(self.selected_partition) {
            Some(p) => p,
            None => return,
        };

        screen.clear();
        screen.put_str_at(5, 5, "=== DELETE PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(
            5,
            7,
            "WARNING: This will delete the partition!",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        screen.put_str_at(5, 9, "Index: ", EFI_GREEN, EFI_BLACK);
        let mut idx_buf = [0u8; 8];
        let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
        screen.put_str_at(
            12,
            9,
            core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"),
            EFI_GREEN,
            EFI_BLACK,
        );

        screen.put_str_at(5, 10, "Type: ", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(11, 10, partition.type_name(), EFI_GREEN, EFI_BLACK);

        screen.put_str_at(5, 11, "Size: ", EFI_GREEN, EFI_BLACK);
        let mut size_buf = [0u8; 16];
        let size_len = Self::format_number(partition.size_mb(), &mut size_buf);
        screen.put_str_at(
            11,
            11,
            core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"),
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(11 + size_len, 11, " MB", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(
            5,
            14,
            "Press Y to confirm, any other key to cancel",
            EFI_DARKGREEN,
            EFI_BLACK,
        );

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        screen.clear();
        screen.put_str_at(5, 5, "Deleting partition...", EFI_LIGHTGREEN, EFI_BLACK);

        // Get disk access
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to access disk",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
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

        // Use core module to delete partition (by GPT entry index, not our display index)
        match gpt_ops::delete_partition(adapter, partition.index as usize) {
            Ok(()) => {
                screen.put_str_at(5, 7, "Partition deleted!", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error",
                    gpt_ops::GptError::PartitionNotFound => "Partition not found",
                    _ => "Unknown error",
                };
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to delete partition",
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
