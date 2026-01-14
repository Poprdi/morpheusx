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
            let title = "=== SHRINK PARTITION ===";
            screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);
            let msg = "No partitions to shrink";
            screen.put_str_at(screen.center_x(msg.len()), 7, msg, EFI_GREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
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
        let content_width = 50;
        let content_x = screen.center_x(content_width);
        let mut textbox = TextBox::new(content_x + 15, 12, 12);
        textbox.selected = true;

        loop {
            screen.clear();
            let title = "=== SHRINK PARTITION ===";
            screen.put_str_at(screen.center_x(title.len()), 3, title, EFI_LIGHTGREEN, EFI_BLACK);

            let warn1 = "WARNING: Shrinking can cause data loss!";
            screen.put_str_at(screen.center_x(warn1.len()), 5, warn1, EFI_LIGHTGREEN, EFI_BLACK);
            let warn2 = "Make sure filesystem is resized first!";
            screen.put_str_at(screen.center_x(warn2.len()), 6, warn2, EFI_LIGHTGREEN, EFI_BLACK);

            screen.put_str_at(content_x, 8, "Index: ", EFI_GREEN, EFI_BLACK);
            let mut idx_buf = [0u8; 8];
            let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
            screen.put_str_at(content_x + 7, 8, core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);

            screen.put_str_at(content_x, 9, "Type: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(content_x + 6, 9, partition.type_name(), EFI_GREEN, EFI_BLACK);

            screen.put_str_at(content_x, 10, "Current size: ", EFI_GREEN, EFI_BLACK);
            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(current_size_mb, &mut size_buf);
            screen.put_str_at(content_x + 14, 10, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
            screen.put_str_at(content_x + 14 + size_len, 10, " MB", EFI_GREEN, EFI_BLACK);

            screen.put_str_at(content_x, 12, "New size (MB): ", EFI_GREEN, EFI_BLACK);
            textbox.render(screen);

            let hint = "Enter new size (must be smaller than current)";
            screen.put_str_at(screen.center_x(hint.len()), 15, hint, EFI_DARKGREEN, EFI_BLACK);
            let help = "[ENTER] Shrink | [ESC] Cancel";
            screen.put_str_at(screen.center_x(help.len()), 17, help, EFI_DARKGREEN, EFI_BLACK);

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
            let title = "=== SHRINK PARTITION ===";
            screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);
            let err = "ERROR: No size entered";
            screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
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
            let title = "=== SHRINK PARTITION ===";
            screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);
            let err = "ERROR: Invalid size";
            screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
            let hint = "New size must be smaller than current size";
            screen.put_str_at(screen.center_x(hint.len()), 9, hint, EFI_GREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }

        // Step 2: Confirmation
        screen.clear();
        let title = "=== CONFIRM SHRINK ===";
        screen.put_str_at(screen.center_x(title.len()), 5, title, EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(content_x, 7, "Current size: ", EFI_GREEN, EFI_BLACK);
        let mut curr_buf = [0u8; 16];
        let curr_len = Self::format_number(current_size_mb, &mut curr_buf);
        screen.put_str_at(content_x + 14, 7, core::str::from_utf8(&curr_buf[..curr_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
        screen.put_str_at(content_x + 14 + curr_len, 7, " MB", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(content_x, 8, "New size:     ", EFI_GREEN, EFI_BLACK);
        let mut new_buf = [0u8; 16];
        let new_len = Self::format_number(new_size_mb, &mut new_buf);
        screen.put_str_at(content_x + 14, 8, core::str::from_utf8(&new_buf[..new_len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(content_x + 14 + new_len, 8, " MB", EFI_GREEN, EFI_BLACK);

        let confirm = "Press Y to confirm, any other key to cancel";
        screen.put_str_at(screen.center_x(confirm.len()), 11, confirm, EFI_DARKGREEN, EFI_BLACK);

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        // Step 3: Perform shrink
        screen.clear();
        let shrinking = "Shrinking partition...";
        screen.put_str_at(screen.center_x(shrinking.len()), 5, shrinking, EFI_LIGHTGREEN, EFI_BLACK);

        // Get disk access
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                let err = "ERROR: Failed to access disk";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
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

        // Use core module to shrink partition
        match gpt_ops::shrink_partition(adapter, partition.index as usize, new_size_mb) {
            Ok(()) => {
                let success = "Partition shrunk successfully!";
                screen.put_str_at(screen.center_x(success.len()), 7, success, EFI_GREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 9, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error",
                    gpt_ops::GptError::PartitionNotFound => "Partition not found",
                    gpt_ops::GptError::InvalidSize => "Invalid size",
                    _ => "Unknown error",
                };
                let err = "ERROR: Failed to shrink partition";
                screen.put_str_at(screen.center_x(err.len()), 7, err, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(screen.center_x(err_msg.len()), 9, err_msg, EFI_GREEN, EFI_BLACK);
                let cont = "Press any key...";
                screen.put_str_at(screen.center_x(cont.len()), 11, cont, EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
}
