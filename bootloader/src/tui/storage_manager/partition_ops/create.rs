use super::super::StorageManager;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::tui::widgets::textbox::TextBox;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::gpt_ops;

impl StorageManager {
    pub(in super::super) fn create_partition_ui(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        // Get disk access
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.clear();
                let title = "=== CREATE PARTITION ===";
                screen.put_str_at(
                    screen.center_x(title.len()),
                    5,
                    title,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let err = "ERROR: Failed to access disk";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    8,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    10,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        let block_io = unsafe { &mut *block_io_ptr };
        let media = unsafe { &*block_io.media };
        let block_size = media.block_size as usize;

        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.clear();
                let title = "=== CREATE PARTITION ===";
                screen.put_str_at(
                    screen.center_x(title.len()),
                    5,
                    title,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let err = "ERROR: Unsupported block size";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    8,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    10,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        // Find free space
        let free_regions = match gpt_ops::find_free_space(adapter, block_size) {
            Ok(regions) => regions,
            Err(_) => {
                screen.clear();
                let title = "=== CREATE PARTITION ===";
                screen.put_str_at(
                    screen.center_x(title.len()),
                    5,
                    title,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let err = "ERROR: Failed to analyze disk";
                screen.put_str_at(
                    screen.center_x(err.len()),
                    8,
                    err,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
                let cont = "Press any key...";
                screen.put_str_at(
                    screen.center_x(cont.len()),
                    10,
                    cont,
                    EFI_DARKGREEN,
                    EFI_BLACK,
                );
                keyboard.wait_for_key();
                return;
            }
        };

        let region = free_regions.iter().find(|r| r.is_some()).and_then(|r| *r);

        if region.is_none() {
            screen.clear();
            let title = "=== CREATE PARTITION ===";
            screen.put_str_at(
                screen.center_x(title.len()),
                5,
                title,
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            let msg = "No free space available";
            screen.put_str_at(screen.center_x(msg.len()), 8, msg, EFI_GREEN, EFI_BLACK);
            let cont = "Press any key...";
            screen.put_str_at(
                screen.center_x(cont.len()),
                10,
                cont,
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            keyboard.wait_for_key();
            return;
        }

        let region = region.unwrap();
        let size_mb = region.size_mb();

        // Step 1: Select partition type
        let mut selected_type = 0;
        let type_names = ["EFI System", "Linux Filesystem", "Linux Swap"];

        loop {
            screen.clear();
            let title = "=== CREATE PARTITION ===";
            screen.put_str_at(
                screen.center_x(title.len()),
                3,
                title,
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );

            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(size_mb, &mut size_buf);
            let size_str = core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?");
            let avail_line = "Available space:     MB";
            let avail_x = screen.center_x(avail_line.len() + size_len);
            screen.put_str_at(avail_x, 5, "Available space: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(avail_x + 17, 5, size_str, EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(avail_x + 17 + size_len, 5, " MB", EFI_GREEN, EFI_BLACK);

            let select_msg = "Select partition type:";
            screen.put_str_at(
                screen.center_x(select_msg.len()),
                8,
                select_msg,
                EFI_GREEN,
                EFI_BLACK,
            );

            for i in 0..3 {
                let y = 10 + i;
                let marker = if i == selected_type { ">" } else { " " };
                let color = if i == selected_type {
                    EFI_LIGHTGREEN
                } else {
                    EFI_GREEN
                };
                let type_line_len = 2 + type_names[i].len();
                let type_x = screen.center_x(type_line_len);

                screen.put_str_at(type_x, y, marker, color, EFI_BLACK);
                screen.put_str_at(type_x + 2, y, type_names[i], color, EFI_BLACK);
            }

            let help = "[UP/DOWN] Navigate | [ENTER] Select | [ESC] Cancel";
            screen.put_str_at(
                screen.center_x(help.len()),
                15,
                help,
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            let key = keyboard.wait_for_key();

            if key.scan_code == 0x01 && selected_type > 0 {
                selected_type -= 1;
            } else if key.scan_code == 0x02 && selected_type < 2 {
                selected_type += 1;
            } else if key.scan_code == 0 && key.unicode_char == 0x000D {
                break; // Selected
            } else if key.scan_code == 0x17 {
                return; // Cancelled
            }
        }

        let partition_type = match selected_type {
            0 => morpheus_core::disk::partition::PartitionType::EfiSystem,
            1 => morpheus_core::disk::partition::PartitionType::LinuxFilesystem,
            2 => morpheus_core::disk::partition::PartitionType::LinuxSwap,
            _ => return,
        };

        // Step 2: Enter size - calculate centered position for textbox
        let content_width = 50;
        let content_x = screen.center_x(content_width);
        let mut textbox = TextBox::new(content_x + 12, 10, 12);
        textbox.selected = true;

        loop {
            screen.clear();
            let title = "=== PARTITION SIZE ===";
            screen.put_str_at(
                screen.center_x(title.len()),
                3,
                title,
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );

            screen.put_str_at(content_x, 5, "Type: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(
                content_x + 6,
                5,
                type_names[selected_type],
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );

            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(size_mb, &mut size_buf);
            screen.put_str_at(content_x, 7, "Available space: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(
                content_x + 17,
                7,
                core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(content_x + 17 + size_len, 7, " MB", EFI_GREEN, EFI_BLACK);

            screen.put_str_at(content_x, 10, "Size (MB): ", EFI_GREEN, EFI_BLACK);
            textbox.render(screen);

            let hint = "Enter size in MB or leave empty for all space";
            screen.put_str_at(
                screen.center_x(hint.len()),
                13,
                hint,
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            let help = "[ENTER] Create | [ESC] Cancel";
            screen.put_str_at(
                screen.center_x(help.len()),
                15,
                help,
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            let key = keyboard.wait_for_key();

            if key.scan_code == 0 && key.unicode_char == 0x000D {
                break; // Confirm
            } else if key.scan_code == 0x17 {
                return; // Cancel
            } else if key.scan_code == 0 && key.unicode_char == 0x0008 {
                textbox.backspace(); // Backspace
            } else if key.unicode_char >= b'0' as u16 && key.unicode_char <= b'9' as u16 {
                textbox.add_char(key.unicode_char as u8);
            }
        }

        // Parse size or use all
        let end_lba = if textbox.length == 0 {
            region.end_lba
        } else {
            let size_text = textbox.get_text();
            let mut requested_mb = 0u64;

            for byte in size_text.bytes() {
                if (b'0'..=b'9').contains(&byte) {
                    requested_mb = requested_mb * 10 + (byte - b'0') as u64;
                }
            }

            if requested_mb == 0 {
                region.end_lba
            } else {
                let requested_lba = (requested_mb * 1024 * 1024) / 512;
                let calculated_end = region.start_lba + requested_lba - 1;

                if calculated_end <= region.end_lba {
                    calculated_end
                } else {
                    region.end_lba
                }
            }
        };

        // Create partition
        screen.clear();
        let creating = "Creating partition...";
        screen.put_str_at(
            screen.center_x(creating.len()),
            5,
            creating,
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
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

        match gpt_ops::create_partition(adapter, partition_type, region.start_lba, end_lba) {
            Ok(()) => {
                let success = "Partition created successfully!";
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
            Err(_) => {
                let err = "ERROR: Failed to create partition";
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
            }
        }
    }
}
