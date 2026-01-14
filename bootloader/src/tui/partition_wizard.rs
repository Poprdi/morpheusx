// Partition creation/deletion wizard UI

use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGREEN, EFI_BLACK};
use crate::tui::input::Keyboard;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::partition::PartitionType;
use morpheus_core::disk::gpt_ops;

// Box constants
const BOX_WIDTH: usize = 77;
const EMPTY_LINE: &str = "|                                                                           |";
const TOP_BORDER: &str = "+===========================================================================+";
const BOTTOM_BORDER: &str = "+===========================================================================+";
const DIVIDER: &str = "+---------------------------------------------------------------------------+";

pub struct PartitionWizard {
    disk_index: usize,
}

impl PartitionWizard {
    pub fn new(disk_index: usize) -> Self {
        Self { disk_index }
    }
    
    pub fn run_create(&self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) -> bool {
        screen.clear();
        
        let x = screen.center_x(BOX_WIDTH);
        let y = screen.center_y(20);
        let mut current_y = y;

        // Top border
        screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Title
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let title = "CREATE NEW PARTITION";
        let padding = (75 - title.len()) / 2;
        screen.put_str_at(x + 1 + padding, current_y, title, EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Get disk protocol
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
                let msg = "ERROR: Failed to access disk";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y, msg, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
                current_y += 1;
                
                screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                current_y += 1;
                
                screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
                let msg = "Press any key...";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y, msg, EFI_DARKGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
                current_y += 1;
                
                screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                current_y += 1;
                screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
                
                keyboard.wait_for_key();
                return false;
            }
        };
        
        let block_io = unsafe { &mut *block_io_ptr };
        let media = unsafe { &*block_io.media };
        let block_size = media.block_size as usize;
        
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
                let msg = "ERROR: Unsupported block size";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y, msg, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
                current_y += 1;
                
                screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                current_y += 1;
                screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
                
                keyboard.wait_for_key();
                return false;
            }
        };
        
        // Find free space
        let free_regions = match gpt_ops::find_free_space(adapter, block_size) {
            Ok(regions) => regions,
            Err(_) => {
                screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
                let msg = "ERROR: Failed to analyze disk";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y, msg, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
                current_y += 1;
                
                screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                current_y += 1;
                screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
                
                keyboard.wait_for_key();
                return false;
            }
        };
        
        if free_regions.is_empty() {
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            let msg = "No free space on disk";
            let padding = (75 - msg.len()) / 2;
            screen.put_str_at(x + 1 + padding, current_y, msg, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;
            
            screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
            current_y += 1;
            screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
            
            keyboard.wait_for_key();
            return false;
        }
        
        // Show free space
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        let region = &free_regions[0];
        let size_mb = region.size_lba() * 512 / 1024 / 1024;
        
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let mut size_str = [0u8; 32];
        let size_len = format_number(size_mb, &mut size_str);
        let size_text = core::str::from_utf8(&size_str[..size_len]).unwrap_or("?");
        let msg = alloc::format!("Free space available: {} MB", size_text);
        let padding = (75 - msg.len()) / 2;
        screen.put_str_at(x + 1 + padding, current_y, &msg, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Select partition type title
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let msg = "Select partition type:";
        let padding = (75 - msg.len()) / 2;
        screen.put_str_at(x + 1 + padding, current_y, msg, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Options
        let options = [
            "[E] EFI System Partition",
            "[L] Linux Filesystem",
            "[S] Linux Swap",
            "[ESC] Cancel",
        ];
        
        for opt in options.iter() {
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            let padding = (75 - opt.len()) / 2;
            screen.put_str_at(x + 1 + padding, current_y, opt, EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;
        }
        
        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;
        
        // Bottom border
        screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
        
        let key = keyboard.wait_for_key();
        
        let partition_type = match key.unicode_char {
            0x0065 | 0x0045 => PartitionType::EfiSystem,
            0x006C | 0x004C => PartitionType::LinuxFilesystem,
            0x0073 | 0x0053 => PartitionType::LinuxSwap,
            _ => return false,
        };
        
        // Create partition - show progress
        screen.clear();
        let current_y = screen.center_y(10);
        let x = screen.center_x(BOX_WIDTH);
        
        screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, current_y + 1, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(x, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
        let msg = "Creating partition...";
        let padding = (75 - msg.len()) / 2;
        screen.put_str_at(x + 1 + padding, current_y + 2, msg, EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
        
        screen.put_str_at(x, current_y + 3, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, current_y + 4, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
        
        // Get fresh adapter
        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                return false;
            }
        };
        
        match gpt_ops::create_partition(adapter, region.start_lba, region.end_lba, partition_type) {
            Ok(()) => {
                screen.clear();
                let current_y = screen.center_y(10);
                
                screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x, current_y + 1, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
                let msg = "Partition created successfully!";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y + 2, msg, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 3, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 4, "|", EFI_GREEN, EFI_BLACK);
                let msg = "Press any key...";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y + 4, msg, EFI_DARKGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y + 4, "|", EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 5, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x, current_y + 6, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
                
                keyboard.wait_for_key();
                true
            }
            Err(_) => {
                screen.clear();
                let current_y = screen.center_y(10);
                
                screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x, current_y + 1, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
                let msg = "ERROR: Failed to create partition";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y + 2, msg, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y + 2, "|", EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 3, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 4, "|", EFI_GREEN, EFI_BLACK);
                let msg = "Press any key...";
                let padding = (75 - msg.len()) / 2;
                screen.put_str_at(x + 1 + padding, current_y + 4, msg, EFI_DARKGREEN, EFI_BLACK);
                screen.put_str_at(x + 76, current_y + 4, "|", EFI_GREEN, EFI_BLACK);
                
                screen.put_str_at(x, current_y + 5, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x, current_y + 6, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
                
                keyboard.wait_for_key();
                false
            }
        }
    }
}

fn format_number(num: u64, buf: &mut [u8]) -> usize {
    if num == 0 {
        buf[0] = b'0';
        return 1;
    }
    
    let mut n = num;
    let mut digits = [0u8; 20];
    let mut count = 0;
    
    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }
    
    for i in 0..count {
        if i < buf.len() {
            buf[i] = digits[count - 1 - i];
        }
    }
    
    count
}
