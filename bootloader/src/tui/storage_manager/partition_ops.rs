use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGREEN, EFI_BLACK};
use crate::tui::input::Keyboard;
use crate::tui::widgets::textbox::TextBox;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::gpt_ops;
use super::StorageManager;

impl StorageManager {
    pub(super) fn create_partition_ui(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
        // Get disk access
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.current_disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.clear();
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
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
                screen.put_str_at(5, 7, "ERROR: Unsupported block size", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };
        
        // Find free space
        let free_regions = match gpt_ops::find_free_space(adapter, block_size) {
            Ok(regions) => regions,
            Err(_) => {
                screen.clear();
                screen.put_str_at(5, 7, "ERROR: Failed to analyze disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };
        
        let region = free_regions.iter().find(|r| r.is_some()).and_then(|r| *r);
        
        if region.is_none() {
            screen.clear();
            screen.put_str_at(5, 7, "No free space available", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
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
            screen.put_str_at(5, 3, "=== CREATE PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);
            
            screen.put_str_at(5, 5, "Available space: ", EFI_GREEN, EFI_BLACK);
            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(size_mb, &mut size_buf);
            screen.put_str_at(22, 5, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(22 + size_len, 5, " MB", EFI_GREEN, EFI_BLACK);
            
            screen.put_str_at(5, 8, "Select partition type:", EFI_GREEN, EFI_BLACK);
            
            for i in 0..3 {
                let y = 10 + i;
                let marker = if i == selected_type { ">" } else { " " };
                let color = if i == selected_type { EFI_LIGHTGREEN } else { EFI_GREEN };
                
                screen.put_str_at(7, y, marker, color, EFI_BLACK);
                screen.put_str_at(9, y, type_names[i], color, EFI_BLACK);
            }
            
            screen.put_str_at(5, 15, "[UP/DOWN] Navigate | [ENTER] Select | [ESC] Cancel", EFI_DARKGREEN, EFI_BLACK);
            
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
        
        // Step 2: Enter size
        let mut textbox = TextBox::new(22, 10, 12);
        textbox.selected = true;
        
        loop {
            screen.clear();
            screen.put_str_at(5, 3, "=== PARTITION SIZE ===", EFI_LIGHTGREEN, EFI_BLACK);
            
            screen.put_str_at(5, 5, "Type: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(11, 5, type_names[selected_type], EFI_LIGHTGREEN, EFI_BLACK);
            
            screen.put_str_at(5, 7, "Available space: ", EFI_GREEN, EFI_BLACK);
            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(size_mb, &mut size_buf);
            screen.put_str_at(22, 7, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(22 + size_len, 7, " MB", EFI_GREEN, EFI_BLACK);
            
            screen.put_str_at(5, 10, "Size (MB):     ", EFI_GREEN, EFI_BLACK);
            textbox.render(screen);
            
            screen.put_str_at(5, 13, "Enter size in MB or leave empty for all space", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(5, 15, "[ENTER] Create | [ESC] Cancel", EFI_DARKGREEN, EFI_BLACK);
            
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
                if byte >= b'0' && byte <= b'9' {
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
        screen.put_str_at(5, 5, "Creating partition...", EFI_LIGHTGREEN, EFI_BLACK);
        
        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };
        
        match gpt_ops::create_partition(adapter, partition_type, region.start_lba, end_lba) {
            Ok(()) => {
                screen.put_str_at(5, 7, "Partition created successfully!", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to create partition", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
    
    pub(super) fn delete_partition_ui(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
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
        screen.put_str_at(5, 7, "WARNING: This will delete the partition!", EFI_LIGHTGREEN, EFI_BLACK);
        
        screen.put_str_at(5, 9, "Index: ", EFI_GREEN, EFI_BLACK);
        let mut idx_buf = [0u8; 8];
        let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
        screen.put_str_at(12, 9, core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(5, 10, "Type: ", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(11, 10, partition.type_name(), EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(5, 11, "Size: ", EFI_GREEN, EFI_BLACK);
        let mut size_buf = [0u8; 16];
        let size_len = Self::format_number(partition.size_mb(), &mut size_buf);
        screen.put_str_at(11, 11, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
        screen.put_str_at(11 + size_len, 11, " MB", EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(5, 14, "Press Y to confirm, any other key to cancel", EFI_DARKGREEN, EFI_BLACK);
        
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
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };
        
        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to create adapter", EFI_LIGHTGREEN, EFI_BLACK);
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
                screen.put_str_at(5, 7, "ERROR: Failed to delete partition", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, err_msg, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
    
    pub(super) fn shrink_partition_ui(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
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
            
            screen.put_str_at(5, 5, "WARNING: Shrinking can cause data loss!", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(5, 6, "Make sure filesystem is resized first!", EFI_LIGHTGREEN, EFI_BLACK);
            
            screen.put_str_at(5, 8, "Index: ", EFI_GREEN, EFI_BLACK);
            let mut idx_buf = [0u8; 8];
            let idx_len = Self::format_number(partition.index as u64, &mut idx_buf);
            screen.put_str_at(12, 8, core::str::from_utf8(&idx_buf[..idx_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
            
            screen.put_str_at(5, 9, "Type: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(11, 9, partition.type_name(), EFI_GREEN, EFI_BLACK);
            
            screen.put_str_at(5, 10, "Current size: ", EFI_GREEN, EFI_BLACK);
            let mut size_buf = [0u8; 16];
            let size_len = Self::format_number(current_size_mb, &mut size_buf);
            screen.put_str_at(19, 10, core::str::from_utf8(&size_buf[..size_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
            screen.put_str_at(19 + size_len, 10, " MB", EFI_GREEN, EFI_BLACK);
            
            screen.put_str_at(5, 12, "New size (MB): ", EFI_GREEN, EFI_BLACK);
            textbox.render(screen);
            
            screen.put_str_at(5, 15, "Enter new size (must be smaller than current)", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(5, 17, "[ENTER] Shrink | [ESC] Cancel", EFI_DARKGREEN, EFI_BLACK);
            
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
            if byte >= b'0' && byte <= b'9' {
                new_size_mb = new_size_mb * 10 + (byte - b'0') as u64;
            }
        }
        
        if new_size_mb == 0 || new_size_mb >= current_size_mb {
            screen.clear();
            screen.put_str_at(5, 5, "ERROR: Invalid size", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(5, 7, "New size must be smaller than current size", EFI_GREEN, EFI_BLACK);
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
        screen.put_str_at(19, 7, core::str::from_utf8(&curr_buf[..curr_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
        screen.put_str_at(19 + curr_len, 7, " MB", EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(5, 8, "New size:     ", EFI_GREEN, EFI_BLACK);
        let mut new_buf = [0u8; 16];
        let new_len = Self::format_number(new_size_mb, &mut new_buf);
        screen.put_str_at(19, 8, core::str::from_utf8(&new_buf[..new_len]).unwrap_or("?"), EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(19 + new_len, 8, " MB", EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(5, 11, "Press Y to confirm, any other key to cancel", EFI_DARKGREEN, EFI_BLACK);
        
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
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return;
            }
        };
        
        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to create adapter", EFI_LIGHTGREEN, EFI_BLACK);
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
                screen.put_str_at(5, 7, "ERROR: Failed to shrink partition", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, err_msg, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 11, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
            }
        }
    }
}
