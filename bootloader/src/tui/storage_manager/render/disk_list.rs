use super::super::StorageManager;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};

impl StorageManager {
    pub(in super::super) fn render_disk_list(&self, screen: &mut Screen) {
        let title = "=== STORAGE DEVICES ===";
        let x = screen.center_x(title.len());
        screen.put_str_at(x, 2, title, EFI_LIGHTGREEN, EFI_BLACK);

        let disk_count = self.disk_manager.disk_count();
        if disk_count == 0 {
            let msg = "No storage devices found";
            screen.put_str_at(
                screen.center_x(msg.len()),
                5,
                msg,
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            let help = "Press ESC to return";
            screen.put_str_at(
                screen.center_x(help.len()),
                7,
                help,
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            return;
        }

        let prompt = "Select a disk to view partitions:";
        screen.put_str_at(
            screen.center_x(prompt.len()),
            4,
            prompt,
            EFI_GREEN,
            EFI_BLACK,
        );

        // Table with dynamic centering
        let table_width = 70;
        let table_x = screen.center_x(table_width);
        let table_y = 6;

        screen.put_str_at(table_x, table_y, "IDX", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 7, table_y, "SIZE (MB)", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(
            table_x + 22,
            table_y,
            "BLOCK SIZE",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(table_x + 37, table_y, "TYPE", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 52, table_y, "STATUS", EFI_LIGHTGREEN, EFI_BLACK);

        let sep = "======================================================================";
        screen.put_str_at(table_x, table_y + 1, sep, EFI_GREEN, EFI_BLACK);

        let disk_count = self.disk_manager.disk_count();
        for i in 0..disk_count {
            if let Some(disk) = self.disk_manager.get_disk(i) {
                let entry_y = table_y + 2 + i;

                let color = if i == self.selected_disk {
                    EFI_LIGHTGREEN
                } else {
                    EFI_GREEN
                };

                let marker = if i == self.selected_disk { ">" } else { " " };
                screen.put_str_at(table_x - 2, entry_y, marker, color, EFI_BLACK);

                let mut idx_buf = [0u8; 8];
                let idx_len = Self::format_number(i as u64, &mut idx_buf);
                screen.put_str_at(
                    table_x,
                    entry_y,
                    core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"),
                    color,
                    EFI_BLACK,
                );

                let mut size_buf = [0u8; 16];
                let size_len = Self::format_number(disk.size_mb(), &mut size_buf);
                screen.put_str_at(
                    table_x + 7,
                    entry_y,
                    core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"),
                    color,
                    EFI_BLACK,
                );

                let mut bs_buf = [0u8; 16];
                let bs_len = Self::format_number(disk.block_size as u64, &mut bs_buf);
                screen.put_str_at(
                    table_x + 22,
                    entry_y,
                    core::str::from_utf8(&bs_buf[..bs_len]).unwrap_or("?"),
                    color,
                    EFI_BLACK,
                );

                let disk_type = if disk.removable { "Removable" } else { "Fixed" };
                screen.put_str_at(table_x + 37, entry_y, disk_type, color, EFI_BLACK);

                let status = if disk.read_only {
                    "Read-Only"
                } else {
                    "Read/Write"
                };
                screen.put_str_at(table_x + 52, entry_y, status, color, EFI_BLACK);
            }
        }

        let status_y = table_y + 2 + disk_count + 1;
        let help_text = "[UP/DOWN] Navigate | [ENTER] View Partitions | [ESC] Back";
        screen.put_str_at(
            screen.center_x(help_text.len()),
            status_y,
            help_text,
            EFI_DARKGREEN,
            EFI_BLACK,
        );
    }


}
