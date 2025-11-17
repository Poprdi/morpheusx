use super::super::StorageManager;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::tui::widgets::textbox::TextBox;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::gpt_ops;


impl StorageManager {
        pub(in super::super) fn shrink_partition_ui(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        if self.partition_table.count() == 0 {
            screen.clear();
            screen.put_str_at(5, 5, "No partitions to shrink", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(5, 7, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        // Get selected partition info
        let partition = match self.partition_table.get(self.selected_partition) {
            Some(p) => p,
            None => return,
        };

        let current_size_mb = partition.size_mb();

        // Step 1: Show partition info and get new size
        let mut textbox = TextBox::new(22, 12, 12);
        textbox.selected = true;

        loop {
            screen.clear();
            screen.put_str_at(5, 3, "=== SHRINK PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);

            screen.put_str_at(
                5,
                5,
                "WARNING: Shrinking can cause data loss!",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(
                5,
                6,
                "Make sure filesystem is resized first!",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );

            screen.put_str_at(5, 8, "Index: ", EFI_GREEN, EFI_BLACK);
            let mut idx_buf = [0u8; 8];
            let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
            screen.put_str_at(
                12,
                8,
                core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"),
                EFI_GREEN,
                EFI_BLACK,
            );

            screen.put_str_at(5, 9, "Type: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(11, 9, partition.type_name(), EFI_GREEN, EFI_BLACK);

            screen.put_str_at(5, 10, "Current size: ", EFI_GREEN, EFI_BLACK);
            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(current_size_mb, &mut size_buf);
            screen.put_str_at(
                19,
                10,
                core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"),
                EFI_GREEN,
                EFI_BLACK,
            );
            screen.put_str_at(19 + size_len, 10, " MB", EFI_GREEN, EFI_BLACK);

            screen.put_str_at(5, 12, "New size (MB): ", EFI_GREEN, EFI_BLACK);
            textbox.render(screen);

            screen.put_str_at(
                5,
                15,
                "Enter new size (must be smaller than current)",
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(
                5,
                17,
                "[ENTER] Shrink | [ESC] Cancel",
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            let key = keyboard.wait_for_key();

            if key.scan_code == 0 && key.unicode_char == 0x000D {
                break; // Confirm
            } else if key.scan_code == 0x17 {
                return; // Cancel
            } else if key.scan_code == 0 && key.unicode_char == 0x0008 {
                textbox.backspace();
            } else if key.unicode_char >= b'0' as u16 && key.unicode_char <= b'9' as u16 {
                textbox.add_char(key.unicode_char as u8);
            }
        }

        // Parse new size
        if textbox.length == 0 {
            screen.clear();
            screen.put_str_at(5, 5, "ERROR: No size entered", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(5, 7, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        let size_text = textbox.get_text();
        let mut new_size_mb = 0u64;

        for byte in size_text.bytes() {
            if (b'0'..=b'9').contains(&byte) {
                new_size_mb = new_size_mb * 10 + (byte - b'0') as u64;
            }
        }

        if new_size_mb == 0 || new_size_mb >= current_size_mb {
            screen.clear();
            screen.put_str_at(5, 5, "ERROR: Invalid size", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(
                5,
                7,
                "New size must be smaller than current size",
                EFI_GREEN,
                EFI_BLACK,
            );
            screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        // Step 2: Confirmation
        screen.clear();
        screen.put_str_at(5, 5, "=== CONFIRM SHRINK ===", EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(5, 7, "Current size: ", EFI_GREEN, EFI_BLACK);
        let mut curr_buf = [0u8; 16];
        let curr_len = Self::format_number(current_size_mb, &mut curr_buf);
        screen.put_str_at(
            19,
            7,
            core::str::from_utf8(&curr_buf[..curr_len]).unwrap_or("?"),
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(19 + curr_len, 7, " MB", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(5, 8, "New size:     ", EFI_GREEN, EFI_BLACK);
        let mut new_buf = [0u8; 16];
        let new_len = Self::format_number(new_size_mb, &mut new_buf);
        screen.put_str_at(
            19,
            8,
            core::str::from_utf8(&new_buf[..new_len]).unwrap_or("?"),
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(19 + new_len, 8, " MB", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(
            5,
            11,
            "Press Y to confirm, any other key to cancel",
            EFI_DARKGREEN,
            EFI_BLACK,
        );

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        // Step 3: Perform shrink
        screen.clear();
        screen.put_str_at(5, 5, "Shrinking partition...", EFI_LIGHTGREEN, EFI_BLACK);

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

        // Use core module to shrink partition
        match gpt_ops::shrink_partition(adapter, partition.index as usize, new_size_mb) {
            Ok(()) => {
                screen.put_str_at(5, 7, "Partition shrunk successfully!", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error",
                    gpt_ops::GptError::PartitionNotFound => "Partition not found",
                    gpt_ops::GptError::InvalidSize => "Invalid size",
                    _ => "Unknown error",
                };
                screen.put_str_at(
                    5,
                    7,
                    "ERROR: Failed to shrink partition",
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
