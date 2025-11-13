// Partition creation/deletion wizard UI

use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGREEN, EFI_BLACK};
use crate::tui::input::Keyboard;
use crate::uefi::gpt_adapter::UefiBlockIoAdapter;
use crate::BootServices;
use morpheus_core::disk::partition::PartitionType;
use morpheus_core::disk::gpt_ops;

pub struct PartitionWizard {
    disk_index: usize,
}

impl PartitionWizard {
    pub fn new(disk_index: usize) -> Self {
        Self { disk_index }
    }
    
    pub fn run_create(&self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) -> bool {
        screen.clear();
        screen.put_str_at(5, 5, "=== CREATE NEW PARTITION ===", EFI_LIGHTGREEN, EFI_BLACK);
        
        // Get disk protocol
        let block_io_ptr = match crate::uefi::disk::get_disk_protocol(bs, self.disk_index) {
            Ok(ptr) => ptr,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
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
                screen.put_str_at(5, 7, "ERROR: Unsupported block size", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return false;
            }
        };
        
        // Find free space
        let free_regions = match gpt_ops::find_free_space(adapter, block_size) {
            Ok(regions) => regions,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to analyze disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return false;
            }
        };
        
        if free_regions.is_empty() {
            screen.put_str_at(5, 7, "No free space on disk", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return false;
        }
        
        // Show free space
        screen.put_str_at(5, 7, "Free space available:", EFI_GREEN, EFI_BLACK);
        let region = &free_regions[0];
        let size_mb = region.size_lba() * 512 / 1024 / 1024;
        
        let mut size_str = [0u8; 32];
        let size_len = format_number(size_mb, &mut size_str);
        screen.put_str_at(7, 9, core::str::from_utf8(&size_str[..size_len]).unwrap_or("?"), EFI_GREEN, EFI_BLACK);
        screen.put_str_at(7 + size_len, 9, " MB", EFI_GREEN, EFI_BLACK);
        
        // Select partition type
        screen.put_str_at(5, 12, "Select partition type:", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(5, 14, "[E] EFI System Partition", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(5, 15, "[L] Linux Filesystem", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(5, 16, "[S] Linux Swap", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(5, 18, "[ESC] Cancel", EFI_DARKGREEN, EFI_BLACK);
        
        let key = keyboard.wait_for_key();
        
        let partition_type = match key.unicode_char {
            0x0065 | 0x0045 => PartitionType::EfiSystem,
            0x006C | 0x004C => PartitionType::LinuxFilesystem,
            0x0073 | 0x0053 => PartitionType::LinuxSwap,
            _ => return false,
        };
        
        // Create partition
        screen.clear();
        screen.put_str_at(5, 5, "Creating partition...", EFI_LIGHTGREEN, EFI_BLACK);
        
        // Get fresh adapter
        let block_io = unsafe { &mut *block_io_ptr };
        let adapter = match UefiBlockIoAdapter::new(block_io) {
            Ok(a) => a,
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to access disk", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                return false;
            }
        };
        
        match gpt_ops::create_partition(adapter, region.start_lba, region.end_lba, partition_type) {
            Ok(()) => {
                screen.put_str_at(5, 7, "Partition created!", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
                keyboard.wait_for_key();
                true
            }
            Err(_) => {
                screen.put_str_at(5, 7, "ERROR: Failed to create partition", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(5, 9, "Press any key...", EFI_DARKGREEN, EFI_BLACK);
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
