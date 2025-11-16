// Distro launcher - select and boot a kernel

use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};
use alloc::string::String;
use alloc::vec::Vec;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
const MAX_INITRD_BYTES: usize = 32 * 1024 * 1024; // 32 MiB - reasonable for initramfs

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
        morpheus_core::logger::log("DistroLauncher::new() start");
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
                cmdline: String::from("root=/dev/ram0 rw console=ttyS0,115200 console=tty0 debug"),
                initrd: Some(String::from("\\initrds\\initramfs-arch.img")),
            },
            KernelEntry {
                name: String::from("Fedora 6.17.4"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/sda1 ro quiet"),
                initrd: None,
            },
            KernelEntry {
                name: String::from("Fedora + Arch initrd (TEST)"),
                path: String::from("\\kernels\\vmlinuz"),
                cmdline: String::from("root=/dev/ram0 rw console=ttyS0,115200 debug"),
                initrd: Some(String::from("\\initrds\\minimal-test.img")),
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

        morpheus_core::logger::log(
            alloc::format!("Created {} kernel entries", kernels.len()).leak(),
        );

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

            // Kernel name - no allocation, just marker + name separately
            screen.put_str_at(10, y, marker, fg, bg);
            screen.put_str_at(12, y, &kernel.name, fg, bg);

            // Path - no allocation, static prefix + path
            screen.put_str_at(10, y + 1, "  Path: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(18, y + 1, &kernel.path, EFI_DARKGREEN, EFI_BLACK);
        }

        // Bottom instructions
        let bottom_y = screen.height() - 2;
        screen.put_str_at(
            5,
            bottom_y,
            "NOTE: Kernel must exist on ESP partition",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
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
                    morpheus_core::logger::log("enter pressed");
                    let kernel = &self.kernels[self.selected_index];
                    morpheus_core::logger::log("kernel selected");
                    self.boot_kernel(
                        screen,
                        keyboard,
                        boot_services,
                        system_table,
                        image_handle,
                        kernel,
                    );
                    morpheus_core::logger::log("boot_kernel returned");
                    // If we return here, boot failed
                    morpheus_core::logger::log("clearing screen");
                    screen.clear();
                    morpheus_core::logger::log("calling render");
                    self.render(screen);
                    morpheus_core::logger::log("render complete");
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
        screen.put_str_at(5, 10, "Loading kernel...", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(5, 12, "Reading kernel image...", EFI_DARKGREEN, EFI_BLACK);

        morpheus_core::logger::log("kernel read start");
        let kernel_data = match Self::read_file_to_vec(
            boot_services,
            image_handle,
            &kernel.path,
            screen,
            MAX_KERNEL_BYTES,
        ) {
            Ok(data) => {
                screen.put_str_at(5, 13, "Kernel loaded successfully", EFI_GREEN, EFI_BLACK);
                morpheus_core::logger::log("kernel read ok");
                data
            }
            Err(e) => {
                screen.put_str_at(5, 15, "ERROR: Failed to read kernel", EFI_RED, EFI_BLACK);
                morpheus_core::logger::log("kernel read failed");
                Self::dump_logs_to_screen(screen);
                keyboard.wait_for_key();
                return;
            }
        };

        let initrd_data = match &kernel.initrd {
            Some(path) => {
                screen.put_str_at(5, 16, "Initrd: loading...", EFI_DARKGREEN, EFI_BLACK);
                morpheus_core::logger::log("initrd read start");
                match Self::read_file_to_vec(
                    boot_services,
                    image_handle,
                    path,
                    screen,
                    MAX_INITRD_BYTES,
                ) {
                    Ok(data) => {
                        screen.put_str_at(
                            5,
                            17,
                            "Initrd loaded successfully",
                            EFI_GREEN,
                            EFI_BLACK,
                        );
                        morpheus_core::logger::log("initrd read ok");
                        Some(data)
                    }
                    Err(e) => {
                        screen.put_str_at(
                            5,
                            18,
                            "ERROR: Failed to read initrd",
                            EFI_RED,
                            EFI_BLACK,
                        );
                        morpheus_core::logger::log("initrd read failed");
                        Self::dump_logs_to_screen(screen);
                        keyboard.wait_for_key();
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
                screen.put_str_at(
                    5,
                    i,
                    "                                                    ",
                    EFI_BLACK,
                    EFI_BLACK,
                );
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

    fn read_file_to_vec(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
        path: &str,
        screen: &mut Screen,
        max_size: usize,
    ) -> Result<Vec<u8>, &'static str> {
        use crate::uefi::file_system::*;

        unsafe {
            let loaded_image = get_loaded_image(boot_services, image_handle)
                .map_err(|_| "Failed to get loaded image")?;

            let device_handle = (*loaded_image).device_handle;
            let fs_protocol = get_file_system_protocol(boot_services, device_handle)
                .map_err(|_| "Failed to get file system protocol")?;
            let root = open_root_volume(fs_protocol).map_err(|_| "Failed to open root volume")?;

            screen.put_str_at(5, 13, "Opening file...", EFI_DARKGREEN, EFI_BLACK);

            let mut utf16_path = [0u16; 256];
            ascii_to_utf16(path, &mut utf16_path);

            let file = open_file_read(root, &utf16_path).map_err(|status| {
                if status == 0x80000000000000 | 14 {
                    "File not found"
                } else {
                    "Failed to open file"
                }
            })?;

            screen.put_str_at(
                5,
                14,
                "Reading file with UEFI pages...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            morpheus_core::logger::log("uefi page alloc");

            // GRUB approach: allocate UEFI pages directly
            const PAGE_SIZE: usize = 4096;
            let pages_needed = (max_size + PAGE_SIZE - 1) / PAGE_SIZE;

            let mut buffer_addr = 0xFFFFFFFFu64;
            let alloc_type = 1; // EFI_ALLOCATE_MAX_ADDRESS
            let mem_type = 2; // EFI_LOADER_DATA

            let status = (boot_services.allocate_pages)(
                alloc_type,
                mem_type,
                pages_needed,
                &mut buffer_addr,
            );

            if status != 0 {
                morpheus_core::logger::log("uefi alloc failed");
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to allocate UEFI pages for file");
            }

            morpheus_core::logger::log("uefi pages allocated");
            let buffer_ptr = buffer_addr as *mut u8;
            let mut bytes_to_read = max_size;

            let status = ((*file).read)(file, &mut bytes_to_read, buffer_ptr);

            if status != 0 {
                morpheus_core::logger::log("uefi read failed");
                (boot_services.free_pages)(buffer_addr, pages_needed);
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to read file");
            }

            morpheus_core::logger::log("uefi read complete");

            close_file(file).ok();
            close_file(root).ok();

            morpheus_core::logger::log("creating vec from raw parts");
            // DON'T copy to Vec - wrap UEFI buffer in Vec without copying
            // Vec::from_raw_parts takes ownership without allocating/copying
            let result = Vec::from_raw_parts(buffer_ptr, bytes_to_read, max_size);

            morpheus_core::logger::log("file read success");
            Ok(result)
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
        screen.put_str_at(
            5,
            start_line + 2,
            "Press any key to return...",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        keyboard.wait_for_key();
    }

    fn dump_logs_to_screen(screen: &mut Screen) {
        let logs = morpheus_core::logger::get_logs();
        let start_y = 20;

        screen.put_str_at(5, start_y, "=== DEBUG LOGS ===", EFI_LIGHTGREEN, EFI_BLACK);

        for (i, log_entry) in logs.iter().enumerate() {
            let y = start_y + 1 + i;
            if y >= screen.height() - 1 {
                break;
            }

            if let Some(msg) = log_entry {
                screen.put_str_at(7, y, msg, EFI_GREEN, EFI_BLACK);
            }
        }
    }

    fn describe_boot_error(error: &BootError) -> alloc::string::String {
        match error {
            BootError::KernelParse(e) => alloc::format!("Kernel parse failed: {:?}", e),
            BootError::KernelAllocation(e) => alloc::format!("Kernel allocation failed: {:?}", e),
            BootError::KernelLoad(e) => alloc::format!("Kernel load failed: {:?}", e),
            BootError::BootParamsAllocation(e) => {
                alloc::format!("Boot params allocation failed: {:?}", e)
            }
            BootError::CmdlineAllocation(e) => alloc::format!("Cmdline allocation failed: {:?}", e),
            BootError::InitrdAllocation(e) => alloc::format!("Initrd allocation failed: {:?}", e),
            BootError::MemorySnapshot(e) => alloc::format!("Memory map build failed: {:?}", e),
            BootError::ExitBootServices(e) => alloc::format!("ExitBootServices failed: {:?}", e),
        }
    }
}
