// Bootloader installer menu UI

use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGREEN, EFI_BLACK, EFI_CYAN, EFI_WHITE};
use crate::tui::input::Keyboard;
use crate::installer::{self, EspInfo, InstallError};
use crate::BootServices;
use alloc::vec::Vec;
use alloc::string::ToString;
use alloc::format;
use morpheus_persistent::feedback::{FeedbackCollector, FeedbackCategory, FeedbackLevel};
use morpheus_persistent::pe::header::PeHeaders;

pub struct InstallerMenu {
    esp_list: Vec<EspInfo>,
    selected_esp: usize,
    scan_complete: bool,
    image_handle: *mut (),
}

impl InstallerMenu {
    pub fn new(image_handle: *mut ()) -> Self {
        Self {
            esp_list: Vec::new(),
            selected_esp: 0,
            scan_complete: false,
            image_handle,
        }
    }
    
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
        loop {
            self.render(screen, bs);
            
            let key = keyboard.wait_for_key();
            
            match key.scan_code {
                0x01 => { // Up arrow
                    if self.selected_esp > 0 {
                        self.selected_esp -= 1;
                    }
                }
                0x02 => { // Down arrow
                    if self.selected_esp + 1 < self.esp_list.len() {
                        self.selected_esp += 1;
                    }
                }
                0x17 => { // ESC
                    return;
                }
                _ => {
                    if key.unicode_char == b'\r' as u16 || key.unicode_char == b'\n' as u16 {
                        // Enter key - install to selected ESP
                        if !self.esp_list.is_empty() && self.selected_esp < self.esp_list.len() {
                            self.install_to_selected(screen, keyboard, bs);
                        }
                    } else if key.unicode_char == b'c' as u16 || key.unicode_char == b'C' as u16 {
                        // Create new ESP - redirect to storage manager
                        self.show_create_esp_help(screen, keyboard);
                    } else if key.unicode_char == b'r' as u16 || key.unicode_char == b'R' as u16 {
                        // Rescan
                        self.scan_complete = false;
                    }
                }
            }
        }
    }
    
    fn render(&mut self, screen: &mut Screen, bs: &BootServices) {
        screen.clear();
        
        let border_width = 80;
        let start_x = screen.center_x(border_width);
        let start_y = 2;
        
        // Title
        screen.put_str_at(start_x, start_y, "=== BOOTLOADER INSTALLER ===", EFI_LIGHTGREEN, EFI_BLACK);
        
        // Scan for ESPs if not done yet
        if !self.scan_complete {
            screen.put_str_at(start_x, start_y + 2, "Scanning for EFI System Partitions...", EFI_GREEN, EFI_BLACK);
            self.scan_for_esps(bs);
            self.scan_complete = true;
        }
        
        let mut y = start_y + 2;
        
        if self.esp_list.is_empty() {
            screen.put_str_at(start_x, y, "No EFI System Partition found", EFI_LIGHTGREEN, EFI_BLACK);
            y += 2;
            screen.put_str_at(start_x, y, "You can:", EFI_GREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [C] Create new ESP partition", EFI_DARKGREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [R] Rescan for ESPs", EFI_DARKGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(start_x, y, "Found EFI System Partitions:", EFI_LIGHTGREEN, EFI_BLACK);
            y += 2;
            
            // Table header
            screen.put_str_at(start_x, y, "   DISK    PART    SIZE (MB)    STATUS", EFI_GREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "========================================", EFI_GREEN, EFI_BLACK);
            y += 1;
            
            // ESP entries
            for (idx, esp) in self.esp_list.iter().enumerate() {
                let marker = if idx == self.selected_esp { "> " } else { "  " };
                
                let disk_str = esp.disk_index.to_string();
                let part_str = esp.partition_index.to_string();
                let size_str = esp.size_mb.to_string();
                
                let mut line = marker.to_string();
                line.push_str(&disk_str);
                
                // Pad to column 2 (PART)
                while line.len() < 9 {
                    line.push(' ');
                }
                line.push_str(&part_str);
                
                // Pad to column 3 (SIZE)
                while line.len() < 17 {
                    line.push(' ');
                }
                line.push_str(&size_str);
                
                // Pad to column 4 (STATUS)
                while line.len() < 30 {
                    line.push(' ');
                }
                line.push_str("Ready");
                
                let fg = if idx == self.selected_esp { EFI_LIGHTGREEN } else { EFI_GREEN };
                screen.put_str_at(start_x, y, &line, fg, EFI_BLACK);
                y += 1;
            }
            
            y += 1;
            screen.put_str_at(start_x, y, "Options:", EFI_GREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [UP/DOWN] Select ESP", EFI_DARKGREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [ENTER] Install to selected ESP", EFI_DARKGREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [C] Create new ESP partition", EFI_DARKGREEN, EFI_BLACK);
            y += 1;
            screen.put_str_at(start_x, y, "  [R] Rescan for ESPs", EFI_DARKGREEN, EFI_BLACK);
        }
        
        y += 2;
        screen.put_str_at(start_x, y, "[ESC] Back to Main Menu", EFI_DARKGREEN, EFI_BLACK);
    }
    
    fn scan_for_esps(&mut self, bs: &BootServices) {
        self.esp_list.clear();
        
        // Scan all disks for ESPs
        let mut temp_disk_manager = morpheus_core::disk::manager::DiskManager::new();
        if crate::uefi::disk::enumerate_disks(bs, &mut temp_disk_manager).is_err() {
            return;
        }
        
        let disk_count = temp_disk_manager.disk_count();
        
        for disk_idx in 0..disk_count {
            let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, disk_idx) {
                Ok(ptr) => ptr,
                Err(_) => continue,
            };
            
            let block_io = unsafe { &mut *block_io_ptr };
            let media = unsafe { &*block_io.media };
            let block_size = media.block_size as usize;
            
            let mut adapter = match crate::uefi::gpt_adapter::UefiBlockIoAdapter::new(block_io) {
                Ok(a) => a,
                Err(_) => continue,
            };
            
            let mut partition_table = morpheus_core::disk::partition::PartitionTable::new();
            if morpheus_core::disk::gpt_ops::scan_partitions(adapter, &mut partition_table, block_size).is_err() {
                continue;
            }
            
            // Find all ESP partitions on this disk
            for part_idx in 0..partition_table.count() {
                if let Some(part) = partition_table.get(part_idx) {
                    if matches!(part.partition_type, morpheus_core::disk::partition::PartitionType::EfiSystem) {
                        self.esp_list.push(EspInfo {
                            disk_index: disk_idx,
                            partition_index: part_idx,
                            start_lba: part.start_lba,
                            size_mb: part.size_mb(),
                        });
                    }
                }
            }
        }
    }
    
    fn install_to_selected(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
        screen.clear();
        let start_x = 2;
        let mut y = 1;
        
        screen.put_str_at(start_x, y, "=== BOOTLOADER PERSISTENCE INSTALLER ===", EFI_LIGHTGREEN, EFI_BLACK);
        y += 1;
        
        if self.esp_list.is_empty() || self.selected_esp >= self.esp_list.len() {
            screen.put_str_at(start_x, y + 1, "ERROR: No ESP selected", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(start_x, y + 3, "Press any key to return...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }
        
        let esp = &self.esp_list[self.selected_esp];
        
        // Create feedback collector
        let mut feedback = FeedbackCollector::new(50);
        
        // Phase 1: Parse PE headers from running bootloader
        y += 1;
        screen.put_str_at(start_x, y, "--- PE Header Analysis ---", EFI_CYAN, EFI_BLACK);
        y += 1;
        
        feedback.info(FeedbackCategory::PeHeader, "Analyzing running bootloader image...");
        self.render_feedback(screen, &feedback, start_x, &mut y);
        
        let loaded_image = unsafe {
            crate::uefi::file_system::get_loaded_image(bs, self.image_handle)
        };
        
        if loaded_image.is_err() {
            feedback.error(FeedbackCategory::General, "Failed to get LoadedImageProtocol");
            self.render_feedback(screen, &feedback, start_x, &mut y);
            screen.put_str_at(start_x, y + 2, "Press any key to return...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return;
        }
        
        let loaded_image = unsafe { &*loaded_image.unwrap() };
        let image_base = loaded_image.image_base as *const u8;
        let image_size = loaded_image.image_size as usize;
        
        feedback.success(FeedbackCategory::Memory, 
            format!("Image loaded at: 0x{:016X}", image_base as u64));
        feedback.info(FeedbackCategory::Memory, 
            format!("Image size: {} bytes", image_size));
        self.render_feedback(screen, &feedback, start_x, &mut y);
        
        // Parse PE headers
        feedback.info(FeedbackCategory::PeHeader, "Parsing DOS/PE/COFF headers...");
        self.render_feedback(screen, &feedback, start_x, &mut y);
        
        let pe_headers = unsafe { PeHeaders::parse(image_base, image_size) };
        
        match pe_headers {
            Ok(headers) => {
                feedback.success(FeedbackCategory::PeHeader, "PE headers parsed successfully");
                feedback.info(FeedbackCategory::PeHeader, 
                    format!("Architecture: {}", headers.coff.machine_name()));
                feedback.info(FeedbackCategory::PeHeader, 
                    format!("ImageBase (in memory - PATCHED): 0x{:016X}", headers.optional.image_base));
                feedback.info(FeedbackCategory::PeHeader, 
                    format!("Sections: {}", headers.coff.number_of_sections));
                
                self.render_feedback(screen, &feedback, start_x, &mut y);
                
                // Reconstruct original ImageBase by analyzing .reloc section
                feedback.info(FeedbackCategory::Relocation, 
                    "Reverse-engineering original ImageBase...");
                self.render_feedback(screen, &feedback, start_x, &mut y);
                
                let actual_load = image_base as u64;
                let reconstruction = unsafe {
                    headers.reconstruct_original_image_base(image_base, image_size, actual_load)
                };
                
                match reconstruction {
                    Ok((orig_base, valid_count, total_count)) => {
                        feedback.info(FeedbackCategory::Relocation,
                            format!("Tested {} relocation entries", total_count));
                        
                        if valid_count == total_count {
                            feedback.success(FeedbackCategory::Relocation,
                                format!("Original ImageBase: 0x{:016X}", orig_base));
                            feedback.success(FeedbackCategory::Relocation,
                                format!("Validated: {}/{} relocations (100%)", valid_count, total_count));
                        } else {
                            feedback.warning(FeedbackCategory::Relocation,
                                format!("Best guess ImageBase: 0x{:016X}", orig_base));
                            feedback.warning(FeedbackCategory::Relocation,
                                format!("Validated: {}/{} relocations", valid_count, total_count));
                        }
                        
                        self.render_feedback(screen, &feedback, start_x, &mut y);
                        
                        let actual_delta = image_base as u64 - orig_base;
                        if actual_delta == 0 {
                            feedback.success(FeedbackCategory::Relocation,
                                "Loaded at preferred address!");
                        } else {
                            feedback.warning(FeedbackCategory::Relocation,
                                format!("Relocation delta: +0x{:016X}", actual_delta));
                            feedback.info(FeedbackCategory::Relocation,
                                "Will reverse relocations for bootable image");
                        }
                        
                        self.render_feedback(screen, &feedback, start_x, &mut y);
                    }
                    Err(e) => {
                        feedback.error(FeedbackCategory::Relocation,
                            format!("Reconstruction failed: {}", e));
                        self.render_feedback(screen, &feedback, start_x, &mut y);
                    }
                }
                
                // Phase 2: Install (current simple version)
                y += 1;
                screen.put_str_at(start_x, y, "--- Installation ---", EFI_CYAN, EFI_BLACK);
                y += 1;
                
                feedback.info(FeedbackCategory::Storage, "Writing to ESP partition...");
                feedback.debug(FeedbackCategory::Storage, 
                    format!("Target: Disk {} Part {} ({}MB)", 
                        esp.disk_index, esp.partition_index, esp.size_mb));
                self.render_feedback(screen, &feedback, start_x, &mut y);
                
                match installer::install_to_esp(bs, esp, self.image_handle) {
                    Ok(()) => {
                        feedback.success(FeedbackCategory::Storage, 
                            "Bootloader written successfully!");
                        feedback.success(FeedbackCategory::General, 
                            "Installation complete - system is now persistent");
                        self.render_feedback(screen, &feedback, start_x, &mut y);
                    }
                    Err(e) => {
                        feedback.error(FeedbackCategory::Storage, 
                            format!("Installation failed: {:?}", e));
                        self.render_feedback(screen, &feedback, start_x, &mut y);
                    }
                }
            }
            Err(e) => {
                feedback.error(FeedbackCategory::PeHeader, 
                    format!("PE parsing failed: {}", e));
                self.render_feedback(screen, &feedback, start_x, &mut y);
            }
        }
        
        screen.put_str_at(start_x, y + 2, "Press any key to return...", EFI_DARKGREEN, EFI_BLACK);
        keyboard.wait_for_key();
    }
    
    fn render_feedback(&self, screen: &mut Screen, feedback: &FeedbackCollector, start_x: usize, y: &mut usize) {
        // Only show last few messages to avoid overflow
        let messages = feedback.messages();
        let start_idx = if messages.len() > 3 { messages.len() - 3 } else { 0 };
        
        for msg in &messages[start_idx..] {
            let (color, prefix) = match msg.level {
                FeedbackLevel::Info => (EFI_GREEN, "[INFO]"),
                FeedbackLevel::Success => (EFI_LIGHTGREEN, "[OK]"),
                FeedbackLevel::Warning => (EFI_WHITE, "[WARN]"),
                FeedbackLevel::Error => (EFI_WHITE, "[ERR]"),  // Use white for visibility
                FeedbackLevel::Debug => (EFI_DARKGREEN, "[DBG]"),
            };
            
            let line = format!("{} {}", prefix, msg.message);
            screen.put_str_at(start_x, *y, &line, color, EFI_BLACK);
            *y += 1;
            
            // Prevent overflow
            if *y >= screen.height() - 3 {
                break;
            }
        }
    }
    
    fn create_new_esp(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
        screen.clear();
        let start_x = screen.center_x(80);
        
        screen.put_str_at(start_x, 3, "=== CREATE NEW ESP ===", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 5, "This will:", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(start_x, 6, "  - Find free space on Disk 0", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 7, "  - Create 512MB ESP partition", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 8, "  - Format as FAT32", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 9, "  - Verify filesystem integrity", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 11, "[Y] Continue    [N] Cancel", EFI_LIGHTGREEN, EFI_BLACK);
        
        let key = keyboard.wait_for_key();
        if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
            return;
        }
        
        screen.clear();
        screen.put_str_at(start_x, 3, "=== CREATING ESP ===", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 5, "Scanning disk...", EFI_GREEN, EFI_BLACK);
        
        match installer::create_esp_and_install(bs, 0) {
            Ok(esp_info) => {
                screen.put_str_at(start_x, 7, "SUCCESS: ESP created and formatted", EFI_LIGHTGREEN, EFI_BLACK);
                
                let disk_str = esp_info.disk_index.to_string();
                screen.put_str_at(start_x, 9, "  Disk:", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(start_x + 15, 9, &disk_str, EFI_LIGHTGREEN, EFI_BLACK);
                
                let part_str = esp_info.partition_index.to_string();
                screen.put_str_at(start_x, 10, "  Partition:", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(start_x + 15, 10, &part_str, EFI_LIGHTGREEN, EFI_BLACK);
                
                let size_str = esp_info.size_mb.to_string();
                screen.put_str_at(start_x, 11, "  Size:", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(start_x + 15, 11, &size_str, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(start_x + 15 + size_str.len(), 11, " MB", EFI_LIGHTGREEN, EFI_BLACK);
                
                // Add to list and select it
                self.esp_list.push(esp_info);
                self.selected_esp = self.esp_list.len() - 1;
                
                screen.put_str_at(start_x, 13, "ESP ready for installation", EFI_LIGHTGREEN, EFI_BLACK);
            }
            Err(InstallError::NoFreeSpc) => {
                screen.put_str_at(start_x, 7, "ERROR: No free space (need 512MB)", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(start_x, 9, "Free up space using Storage Manager", EFI_GREEN, EFI_BLACK);
            }
            Err(InstallError::FormatFailed) => {
                screen.put_str_at(start_x, 7, "ERROR: Partition created but format failed", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(start_x, 9, "Try formatting manually in Storage Manager", EFI_GREEN, EFI_BLACK);
            }
            Err(InstallError::IoError) => {
                screen.put_str_at(start_x, 7, "ERROR: Failed to create partition", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(start_x, 9, "Disk may be full or GPT corrupted", EFI_GREEN, EFI_BLACK);
            }
            Err(InstallError::ProtocolError) => {
                screen.put_str_at(start_x, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
            }
            Err(_) => {
                screen.put_str_at(start_x, 7, "ERROR: Unknown error occurred", EFI_LIGHTGREEN, EFI_BLACK);
            }
        }
        
        screen.put_str_at(start_x, 17, "Press any key to continue...", EFI_DARKGREEN, EFI_BLACK);
        keyboard.wait_for_key();
        
        // Mark for rescan to show updated list
        self.scan_complete = false;
    }
    
    fn show_create_esp_help(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) {
        screen.clear();
        let start_x = screen.center_x(80);
        
        screen.put_str_at(start_x, 3, "=== CREATE ESP PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 5, "To create an EFI System Partition:", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(start_x, 7, "1. Go to Storage Manager (from main menu)", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 8, "2. Select your target disk", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 9, "3. Create new partition (min 100MB, recommend 512MB)", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 10, "4. Set partition type to 'EFI System'", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 11, "5. Format as FAT32", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(start_x, 12, "6. Return here and rescan [R]", EFI_DARKGREEN, EFI_BLACK);
        
        screen.put_str_at(start_x, 15, "Press any key to return...", EFI_DARKGREEN, EFI_BLACK);
        keyboard.wait_for_key();
    }
}
