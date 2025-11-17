// Distro launcher - select and boot a kernel

use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};
use alloc::string::String;
use alloc::vec::Vec;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

pub struct DistroLauncher {
    pub(super) kernels: Vec<KernelEntry>,
    pub(super) selected_index: usize,
}

pub(super) struct KernelEntry {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) cmdline: String,
    pub(super) initrd: Option<String>,
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
}
