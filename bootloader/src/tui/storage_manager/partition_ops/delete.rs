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
            let title = "=== DELETE PARTITION ===";
            screen.put_str_at(
                screen.center_x(title.len()),
                5,
                title,
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            let msg = "No partitions to delete";
            screen.put_str_at(screen.center_x(msg.len()), 7, msg, EFI_GREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(
                screen.center_x(cont.len()),
                9,
                cont,
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            keyboard.wait_for_key();
            return;
        }

        // Get selected partition info
        let partition = match self.partition_table.get(self.selected_partition) {
            Some(p) => p,
            None => return,
        };

        screen.clear();
        let title = "=== DELETE PARTITION ===";
        screen.put_str_at(
            screen.center_x(title.len()),
            5,
            title,
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        let warn = "WARNING: This will delete the partition!";
        screen.put_str_at(
            screen.center_x(warn.len()),
            7,
            warn,
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        let content_width = 40;
        let content_x = screen.center_x(content_width);

        screen.put_str_at(content_x, 9, "Index: ", EFI_GREEN, EFI_BLACK);
        let mut idx_buf = [0u8; 8];
        let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
        screen.put_str_at(
            content_x + 7,
            9,
            core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"),
            EFI_GREEN,
            EFI_BLACK,
        );

        screen.put_str_at(content_x, 10, "Type: ", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(
            content_x + 6,
            10,
            partition.type_name(),
            EFI_GREEN,
            EFI_BLACK,
        );

        screen.put_str_at(content_x, 11, "Size: ", EFI_GREEN, EFI_BLACK);
        let mut size_buf = [0u8; 16];
        let size_len = Self::format_number(partition.size_mb(), &mut size_buf);
        screen.put_str_at(
            content_x + 6,
            11,
            core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"),
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(content_x + 6 + size_len, 11, " MB", EFI_GREEN, EFI_BLACK);

        let confirm = "Press Y to confirm, any other key to cancel";
        screen.put_str_at(
            screen.center_x(confirm.len()),
            14,
            confirm,
            EFI_DARKGREEN,
            EFI_BLACK,
        );

        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }

        screen.clear();
        let deleting = "Deleting partition...";
        screen.put_str_at(
            screen.center_x(deleting.len()),
            5,
            deleting,
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        // Get disk access
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                let err = "ERROR: Failed to access disk";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    7,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    9,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                let err = "ERROR: Failed to create adapter";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    7,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    9,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        // Use core module to delete partition (by GPT entry index, not our display index)
        match gpt_ops::delete_partition(adapter, partition.index as usize) {
            Ok(()) => {
                let success = "Partition deleted!";
                screen.put_str_at(
                    screen.center_x(success.len()),
                    7,
                    success,
                    EFI_GREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    9,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
            }
            Err(e) => {
                let err_msg = match e {
                    gpt_ops::GptError::IoError => "I/O Error",
                    gpt_ops::GptError::PartitionNotFound => "Partition not found",
                    _ => "Unknown error",
                };
                let err = "ERROR: Failed to delete partition";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    7,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                screen.put_str_at(
                    screen.center_x(err_msg.len()),
                    9,
                    err_msg,
                    EFI_GREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    11,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
            }
        }
    }
}
