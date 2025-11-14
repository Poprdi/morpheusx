// Distro launcher - select and boot a kernel

use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_BLACK, EFI_DARKGREEN, EFI_RED};
use crate::tui::input::Keyboard;
use crate::boot::loader::BootError;
use alloc::vec::Vec;
use alloc::string::String;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
const MAX_INITRD_BYTES: usize = 128 * 1024 * 1024; // 128 MiB

pub struct DistroLauncher {
    kernels: Vec<KernelEntry>,
    selected_index: usize,
}

struct KernelEntry {
    name: String,
    path: String,
    cmdline: String,
    initrd: Option<String>,
}

impl DistroLauncher {
    pub fn new() -> Self {
        // For now, hardcode some test kernel paths
        // Later we can scan ESP for vmlinuz files
        let kernels = alloc::vec![
            KernelEntry {
                name: String::from("Bootloader Test (with initrd)"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("console=ttyS0,115200 debug"),
                initrd: Some(String::from("\\initrds\\initramfs-test.img")),
            },
            KernelEntry {
                name: String::from("Arch Linux"),
                path: String::from("\\kernels\\vmlinuz-arch"),
                cmdline: String::from("root=/dev/ram0 rw console=ttyS0,115200 debug init=/usr/bin/bash"),
                initrd: Some(String::from("\\initrds\\initramfs-arch.img")),
            },
            KernelEntry {
                name: String::from("Fedora 6.17.4"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/sda1 ro quiet"),
                initrd: None,
            },
            KernelEntry {
                name: String::from("Fedora (verbose)"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/sda1 ro debug earlyprintk=serial console=ttyS0"),
                initrd: None,
            },
            KernelEntry {
                name: String::from("Test File"),
                path: String::from("\\kernels\\test.efi"),
                cmdline: String::from("test"),
                initrd: None,
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
                    self.boot_kernel(screen, keyboard, boot_services, system_table, image_handle, kernel);
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
        keyboard: &mut Keyboard,
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

        screen.put_str_at(5, 14, "Reading kernel image...", EFI_DARKGREEN, EFI_BLACK);
        morpheus_core::logger::log("kernel read start");
        let kernel_data = match Self::read_file_from_esp(
            boot_services,
            image_handle,
            &kernel.path,
            screen,
            MAX_KERNEL_BYTES,
        ) {
            Ok(data) => {
                screen.put_str_at(5, 15, 
                    &alloc::format!("Kernel loaded: {} bytes", data.len()), 
                    EFI_GREEN, EFI_BLACK);
                morpheus_core::logger::log("kernel read ok");
                data
            }
            Err(e) => {
                let msg = alloc::format!("ERROR: Failed to read kernel: {}", e);
                Self::await_failure(screen, keyboard, 18, &msg, "kernel read failed");
                return;
            }
        };

        let initrd_data = match &kernel.initrd {
            Some(path) => {
                screen.put_str_at(5, 16, &alloc::format!("Initrd: {}", path), EFI_DARKGREEN, EFI_BLACK);
                morpheus_core::logger::log("initrd read start");
                match Self::read_file_from_esp(
                    boot_services,
                    image_handle,
                    path,
                    screen,
                    MAX_INITRD_BYTES,
                ) {
                    Ok(data) => {
                        screen.put_str_at(5, 17, 
                            &alloc::format!("Initrd loaded: {} bytes", data.len()), 
                            EFI_GREEN, EFI_BLACK);
                        morpheus_core::logger::log("initrd read ok");
                        Some(data)
                    }
                    Err(e) => {
                        let msg = alloc::format!("ERROR: Failed to read initrd: {}", e);
                        Self::await_failure(screen, keyboard, 18, &msg, "initrd read failed");
                        return;
                    }
                }
            }
            None => {
                screen.put_str_at(5, 16, "Initrd: none", EFI_DARKGREEN, EFI_BLACK);
                morpheus_core::logger::log("initrd missing");
                None
            }
        };

        screen.put_str_at(5, 18, "Booting...", EFI_LIGHTGREEN, EFI_BLACK);

    // Clear old logs before jumping to the kernel
        // Start boot process in background (it will log as it goes)
        // We can't actually make it background, so we'll just check logs after
        // But for now, let's display logs before the call
        
        // Actually, we need to modify boot_linux_kernel to take a callback
        // For now, let's just do a simpler approach: show logs that were added
        
        // Boot the kernel - this will add logs to the buffer
        // We'll instrument it to show progress
    let boot_result = unsafe {
            // Before calling, clear the screen area for logs
            for i in 18..30 {
                screen.put_str_at(5, i, "                                                    ", EFI_BLACK, EFI_BLACK);
            }
            
            crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                &kernel_data,
                initrd_data.as_deref(),
                &kernel.cmdline,
                screen,
            )
        };

    if let Err(error) = boot_result {
            let detail = Self::describe_boot_error(&error);
            let msg = alloc::format!("ERROR: {}", detail);
            Self::await_failure(screen, keyboard, 18, &msg, "kernel boot failed");
        }
    }

    fn read_file_from_esp(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
        path: &str,
        screen: &mut Screen,
        max_size: usize,
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
            let mut file_buffer = alloc::vec![0u8; max_size];
            let mut read_size = max_size;
            
            // Read file
            let status = ((*file).read)(file, &mut read_size, file_buffer.as_mut_ptr());
            
            close_file(file).ok();
            close_file(root).ok();
            
            if status != 0 {
                return Err("Failed to read kernel file");
            }
            
            // Trim buffer to actual size
            file_buffer.truncate(read_size);
            
            Ok(file_buffer)
        }
    }

    fn await_failure(
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        start_line: usize,
        message: &str,
        log_tag: &'static str,
    ) {
        morpheus_core::logger::log(log_tag);
        screen.put_str_at(5, start_line, message, EFI_RED, EFI_BLACK);
        screen.put_str_at(5, start_line + 2, "Press any key to return...", EFI_DARKGREEN, EFI_BLACK);
        keyboard.wait_for_key();
    }

    fn describe_boot_error(error: &BootError) -> alloc::string::String {
        match error {
            BootError::KernelParse(e) => alloc::format!("Kernel parse failed: {:?}", e),
            BootError::KernelAllocation(e) => alloc::format!("Kernel allocation failed: {:?}", e),
            BootError::KernelLoad(e) => alloc::format!("Kernel load failed: {:?}", e),
            BootError::BootParamsAllocation(e) => alloc::format!("Boot params allocation failed: {:?}", e),
            BootError::CmdlineAllocation(e) => alloc::format!("Cmdline allocation failed: {:?}", e),
            BootError::InitrdAllocation(e) => alloc::format!("Initrd allocation failed: {:?}", e),
            BootError::MemorySnapshot(e) => alloc::format!("Memory map build failed: {:?}", e),
            BootError::ExitBootServices(e) => alloc::format!("ExitBootServices failed: {:?}", e),
        }
    }
}
