use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGREEN, EFI_BLACK, EFI_CYAN};
use super::StorageManager;

impl StorageManager {
    pub(super) fn render_disk_list(&self, screen: &mut Screen) {
        let title = "=== STORAGE DEVICES ===";
        let x = screen.center_x(title.len());
        screen.put_str_at(x, 2, title, EFI_LIGHTGREEN, EFI_BLACK);
        
        let disk_count = self.disk_manager.disk_count();
        if disk_count == 0 {
            let msg = "No storage devices found";
            screen.put_str_at(screen.center_x(msg.len()), 5, msg, EFI_LIGHTGREEN, EFI_BLACK);
            let help = "Press ESC to return";
            screen.put_str_at(screen.center_x(help.len()), 7, help, EFI_DARKGREEN, EFI_BLACK);
            return;
        }
        
        let prompt = "Select a disk to view partitions:";
        screen.put_str_at(screen.center_x(prompt.len()), 4, prompt, EFI_GREEN, EFI_BLACK);
        
        // Table with dynamic centering
        let table_width = 70;
        let table_x = screen.center_x(table_width);
        let table_y = 6;
        
        screen.put_str_at(table_x, table_y, "IDX", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 7, table_y, "SIZE (MB)", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 22, table_y, "BLOCK SIZE", EFI_LIGHTGREEN, EFI_BLACK);
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
                screen.put_str_at(table_x, entry_y, core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"), color, EFI_BLACK);
                
                let mut size_buf = [0u8; 16];
                let size_len = Self::format_number(disk.size_mb(), &mut size_buf);
                screen.put_str_at(table_x + 7, entry_y, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), color, EFI_BLACK);
                
                let mut bs_buf = [0u8; 16];
                let bs_len = Self::format_number(disk.block_size as u64, &mut bs_buf);
                screen.put_str_at(table_x + 22, entry_y, core::str::from_utf8(&bs_buf[..bs_len]).unwrap_or("?"), color, EFI_BLACK);
                
                let disk_type = if disk.removable { "Removable" } else { "Fixed" };
                screen.put_str_at(table_x + 37, entry_y, disk_type, color, EFI_BLACK);
                
                let status = if disk.read_only { "Read-Only" } else { "Read/Write" };
                screen.put_str_at(table_x + 52, entry_y, status, color, EFI_BLACK);
            }
        }
        
        let status_y = table_y + 2 + disk_count + 1;
        let help_text = "[UP/DOWN] Navigate | [ENTER] View Partitions | [ESC] Back";
        screen.put_str_at(screen.center_x(help_text.len()), status_y, help_text, EFI_DARKGREEN, EFI_BLACK);
    }
    
    pub(super) fn render_partition_view(&self, screen: &mut Screen) {
        let title = "=== PARTITION VIEW ===";
        screen.put_str_at(screen.center_x(title.len()), 2, title, EFI_LIGHTGREEN, EFI_BLACK);
        
        let subtitle = "Disk selected - viewing partitions";
        screen.put_str_at(screen.center_x(subtitle.len()), 4, subtitle, EFI_CYAN, EFI_BLACK);
        
        if !self.partition_table.has_gpt {
            let no_gpt = "GPT: Not detected";
            screen.put_str_at(screen.center_x(no_gpt.len()), 5, no_gpt, EFI_LIGHTGREEN, EFI_BLACK);
            let raw_msg = "Raw block device - no partition table";
            screen.put_str_at(screen.center_x(raw_msg.len()), 7, raw_msg, EFI_GREEN, EFI_BLACK);
            let help = "[C] Create GPT | [ESC] Back to disk list";
            screen.put_str_at(screen.center_x(help.len()), 9, help, EFI_DARKGREEN, EFI_BLACK);
            return;
        }
        
        let gpt_ok = "GPT: Present";
        screen.put_str_at(screen.center_x(gpt_ok.len()), 5, gpt_ok, EFI_GREEN, EFI_BLACK);
        
        // Partition table with dynamic centering
        let table_width = 70;
        let table_x = screen.center_x(table_width);
        let table_y = 7;
        
        screen.put_str_at(table_x, table_y, "IDX", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 7, table_y, "TYPE", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 25, table_y, "START LBA", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 39, table_y, "END LBA", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(table_x + 53, table_y, "SIZE (MB)", EFI_LIGHTGREEN, EFI_BLACK);
        
        let separator = "=======================================================================";
        screen.put_str_at(table_x, table_y + 1, separator, EFI_GREEN, EFI_BLACK);
        
        let mut row_count = 0;
        
        if self.partition_table.count() == 0 {
            let empty_msg = "(No partitions - press N to create)";
            screen.put_str_at(screen.center_x(empty_msg.len()), table_y + 3, empty_msg, EFI_DARKGREEN, EFI_BLACK);
        } else {
            for (i, part) in self.partition_table.iter().enumerate() {
                let entry_y = table_y + 2 + i;
            
            let color = if i == self.selected_partition {
                EFI_LIGHTGREEN
            } else {
                EFI_GREEN
            };
            
            let marker = if i == self.selected_partition { ">" } else { " " };
            screen.put_str_at(table_x - 2, entry_y, marker, color, EFI_BLACK);
            
            let mut idx_buf = [0u8; 8];
            let idx_len = Self::format_number(part.index as u64, &mut idx_buf);
            screen.put_str_at(table_x, entry_y, core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"), color, EFI_BLACK);
            
            screen.put_str_at(table_x + 7, entry_y, part.type_name(), color, EFI_BLACK);
            
            let mut start_buf = [0u8; 20];
            let start_len = Self::format_number(part.start_lba, &mut start_buf);
            screen.put_str_at(table_x + 25, entry_y, core::str::from_utf8(&start_buf[..start_len]).unwrap_or("?"), color, EFI_BLACK);
            
            let mut end_buf = [0u8; 20];
            let end_len = Self::format_number(part.end_lba, &mut end_buf);
            screen.put_str_at(table_x + 39, entry_y, core::str::from_utf8(&end_buf[..end_len]).unwrap_or("?"), color, EFI_BLACK);
            
            let mut size_buf = [0u8; 20];
            let size_len = Self::format_number(part.size_mb(), &mut size_buf);
            screen.put_str_at(table_x + 53, entry_y, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), color, EFI_BLACK);
            
            row_count = i + 1;
            }
        }
        
        let status_y = table_y + 2 + row_count + 1;
        let help_text = "[UP/DOWN] Navigate | [N] New | [F] Format | [S] Shrink | [D] Delete | [ESC] Back";
        screen.put_str_at(screen.center_x(help_text.len()), status_y, help_text, EFI_DARKGREEN, EFI_BLACK);
        
        // Display free space (centered)
        let free_y = status_y + 2;
        let free_label = "Unpartitioned: ";
        
        if self.free_space_mb > 0 {
            let mut free_buf = [0u8; 20];
            let free_len = Self::format_number(self.free_space_mb, &mut free_buf);
            let free_str = core::str::from_utf8(&free_buf[..free_len]).unwrap_or("?");
            let suffix = " MB available";
            let total_len = free_label.len() + free_str.len() + suffix.len();
            let free_x = screen.center_x(total_len);
            
            screen.put_str_at(free_x, free_y, free_label, EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(free_x + free_label.len(), free_y, free_str, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(free_x + free_label.len() + free_len, free_y, suffix, EFI_DARKGREEN, EFI_BLACK);
        } else {
            let full_msg = "Unpartitioned: 0 MB (disk full)";
            screen.put_str_at(screen.center_x(full_msg.len()), free_y, full_msg, EFI_DARKGREEN, EFI_BLACK);
        }
    }
}
