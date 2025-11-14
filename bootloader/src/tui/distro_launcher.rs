// Distro launcher - select and boot a kernel

use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_BLACK, EFI_DARKGREEN, EFI_RED};
use crate::tui::input::{Keyboard, InputKey};
use alloc::vec::Vec;
use alloc::string::String;

pub struct DistroLauncher {
    kernels: Vec<KernelEntry>,
    selected_index: usize,
}

struct KernelEntry {
    name: String,
    path: String,
    cmdline: String,
}

impl DistroLauncher {
    pub fn new() -> Self {
        // For now, hardcode some test kernel paths
        // Later we can scan ESP for vmlinuz files
        let kernels = alloc::vec![
            KernelEntry {
                name: String::from("Fedora 6.17.4"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/sda1 ro quiet"),
            },
            KernelEntry {
                name: String::from("Fedora (verbose)"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/sda1 ro debug earlyprintk=serial console=ttyS0"),
            },
            KernelEntry {
                name: String::from("Test File"),
                path: String::from("\\kernels\\test.efi"),
                cmdline: String::from("test"),
            },
        ];

        Self {
            kernels,
            selected_index: 0,
        }
    }

    fn select_next(&mut self) {
        if self.selected_index < self.kernels.len() - 1 {
            self.selected_index += 1;
        }
    }

    fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn render(&self, screen: &mut Screen) {
        let title = "=== DISTRO LAUNCHER ===";
        let title_x = (screen.width() - title.len()) / 2;
        screen.put_str_at(title_x, 2, title, EFI_LIGHTGREEN, EFI_BLACK);

        let info = "Use UP/DOWN to select, ENTER to boot, ESC to return";
        let info_x = (screen.width() - info.len()) / 2;
        screen.put_str_at(info_x, 4, info, EFI_DARKGREEN, EFI_BLACK);

        // Render kernel list
        let start_y = 7;
        for (i, kernel) in self.kernels.iter().enumerate() {
            let y = start_y + (i * 3);
            
            let (fg, bg, marker) = if i == self.selected_index {
                (EFI_BLACK, EFI_LIGHTGREEN, "> ")
            } else {
                (EFI_GREEN, EFI_BLACK, "  ")
            };

            // Kernel name
            let name_line = alloc::format!("{}{}", marker, kernel.name);
            screen.put_str_at(10, y, &name_line, fg, bg);

            // Path (smaller text, not highlighted)
            let path_line = alloc::format!("  Path: {}", kernel.path);
            screen.put_str_at(10, y + 1, &path_line, EFI_DARKGREEN, EFI_BLACK);
        }

        // Bottom instructions
        let bottom_y = screen.height() - 2;
        screen.put_str_at(5, bottom_y, 
            "NOTE: Kernel must exist on ESP partition", 
            EFI_DARKGREEN, EFI_BLACK);
    }

    pub fn run(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
    ) {
        screen.clear();
        self.render(screen);

        loop {
            if let Some(key) = keyboard.read_key() {
                // ESC - return to main menu
                if key.scan_code == 0x17 {
                    return;
                }

                // Up arrow
                if key.scan_code == 0x01 {
                    self.select_prev();
                    screen.clear();
                    self.render(screen);
                }

                // Down arrow
                if key.scan_code == 0x02 {
                    self.select_next();
                    screen.clear();
                    self.render(screen);
                }

                // Enter - boot selected kernel
                if key.unicode_char == 0x0D {
                    let kernel = &self.kernels[self.selected_index];
                    self.boot_kernel(screen, boot_services, system_table, image_handle, kernel);
                    // If we return here, boot failed
                    screen.clear();
                    self.render(screen);
                }
            }
        }
    }

    fn boot_kernel(
        &self,
        screen: &mut Screen,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
        kernel: &KernelEntry,
    ) {
        screen.clear();
        screen.put_str_at(5, 10, 
            &alloc::format!("Loading kernel: {}", kernel.name), 
            EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(5, 12, 
            &alloc::format!("Path: {}", kernel.path), 
            EFI_GREEN, EFI_BLACK);

        // Read kernel from ESP using UEFI File System
        let kernel_data = match Self::read_kernel_from_esp(
            boot_services,
            image_handle,
            &kernel.path,
            screen,
        ) {
            Ok(data) => data,
            Err(e) => {
                screen.put_str_at(5, 15, 
                    &alloc::format!("ERROR: Failed to read kernel: {}", e), 
                    EFI_RED, EFI_BLACK);
                screen.put_str_at(5, 17, 
                    "Press any key to return...", 
                    EFI_DARKGREEN, EFI_BLACK);
                
                let mut kb = Keyboard::new(core::ptr::null_mut());
                kb.wait_for_key();
                return;
            }
        };

        screen.put_str_at(5, 14, 
            &alloc::format!("Kernel loaded: {} bytes", kernel_data.len()), 
            EFI_GREEN, EFI_BLACK);
        screen.put_str_at(5, 16, 
            "Booting...", 
            EFI_LIGHTGREEN, EFI_BLACK);

        // Boot the kernel
        unsafe {
            let _ = crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                &kernel_data,
                &kernel.cmdline,
            );
        }

        // If we get here, boot failed
        screen.put_str_at(5, 18, 
            "ERROR: Boot failed", 
            EFI_RED, EFI_BLACK);
        let mut kb = Keyboard::new(core::ptr::null_mut());
        kb.wait_for_key();
    }

    fn read_kernel_from_esp(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
        path: &str,
        screen: &mut Screen,
    ) -> Result<Vec<u8>, &'static str> {
        use crate::uefi::file_system::*;
        
        unsafe {
            // Get loaded image to find our device
            let loaded_image = get_loaded_image(boot_services, image_handle)
                .map_err(|_| "Failed to get loaded image")?;
            
            let device_handle = (*loaded_image).device_handle;
            
            // Get file system protocol
            let fs_protocol = get_file_system_protocol(boot_services, device_handle)
                .map_err(|_| "Failed to get file system protocol")?;
            
            // Open root volume
            let root = open_root_volume(fs_protocol)
                .map_err(|_| "Failed to open root volume")?;
            
            screen.put_str_at(5, 13, "Opening file...", EFI_DARKGREEN, EFI_BLACK);
            
            // Convert path to UTF-16
            let mut utf16_path = [0u16; 256];
            ascii_to_utf16(path, &mut utf16_path);
            
            // Open file for reading
            let file = open_file_read(root, &utf16_path)
                .map_err(|_| "Failed to open kernel file")?;
            
            // Get file size - read a bit to determine size
            // We'll allocate a large buffer and read
            const MAX_KERNEL_SIZE: usize = 32 * 1024 * 1024; // 32MB max
            let mut kernel_buffer = alloc::vec![0u8; MAX_KERNEL_SIZE];
            let mut read_size = MAX_KERNEL_SIZE;
            
            // Read file
            let status = ((*file).read)(file, &mut read_size, kernel_buffer.as_mut_ptr());
            
            close_file(file).ok();
            close_file(root).ok();
            
            if status != 0 {
                return Err("Failed to read kernel file");
            }
            
            // Trim buffer to actual size
            kernel_buffer.truncate(read_size);
            
            Ok(kernel_buffer)
        }
    }
}
