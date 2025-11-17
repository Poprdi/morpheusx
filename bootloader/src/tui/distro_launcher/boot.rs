// Distro launcher - select and boot a kernel

use super::ui::{DistroLauncher, KernelEntry};
use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};
use alloc::string::String;
use alloc::vec::Vec;

const MAX_KERNEL_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

impl DistroLauncher {
    pub(super) fn boot_kernel(
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

        morpheus_core::logger::log("kernel read start");
        let (kernel_ptr, kernel_size) = match Self::read_file_to_uefi_pages(
            boot_services,
            image_handle,
            &kernel.path,
            screen,
            MAX_KERNEL_BYTES,
            12, // start line for kernel status msgs
        ) {
            Ok((ptr, size)) => {
                morpheus_core::logger::log("kernel read ok");
                (ptr, size)
            }
            Err(e) => {
                screen.put_str_at(5, 18, "ERROR: Failed to read kernel", EFI_RED, EFI_BLACK);
                morpheus_core::logger::log("kernel read failed");
                Self::dump_logs_to_screen(screen);
                keyboard.wait_for_key();
                return;
            }
        };

        // Load initrd using same GRUB-style approach
        morpheus_core::logger::log("BEFORE initrd match");
        let (initrd_ptr, initrd_size) = match &kernel.initrd {
            Some(path) => {
                morpheus_core::logger::log("initrd path found");
                
                match Self::read_file_to_uefi_pages(
                    boot_services,
                    image_handle,
                    path,
                    screen,
                    512 * 1024 * 1024, // 512MB max for initrd
                    19, // start line for initrd status msgs (after kernel section)
                ) {
                    Ok((ptr, size)) => {
                        morpheus_core::logger::log("initrd read ok");
                        (Some(ptr), size)
                    }
                    Err(e) => {
                        screen.put_str_at(
                            5,
                            25,
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
                screen.put_str_at(5, 19, "Initrd: none", EFI_DARKGREEN, EFI_BLACK);
                morpheus_core::logger::log("initrd missing");
                (None, 0)
            }
        };

        screen.put_str_at(5, 18, "Booting...", EFI_LIGHTGREEN, EFI_BLACK);

        // Boot the kernel - convert raw pointers to slices (GRUB style)
        let boot_result = unsafe {
            // Clear screen area for logs
            for i in 18..30 {
                screen.put_str_at(
                    5,
                    i,
                    "                                                    ",
                    EFI_BLACK,
                    EFI_BLACK,
                );
            }

            // Convert pointers to slices for boot call
            let kernel_slice = core::slice::from_raw_parts(kernel_ptr, kernel_size);
            let initrd_slice = initrd_ptr.map(|ptr| core::slice::from_raw_parts(ptr, initrd_size));

            crate::boot::loader::boot_linux_kernel(
                boot_services,
                system_table,
                image_handle,
                kernel_slice,
                initrd_slice,
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
    fn read_file_to_uefi_pages(
        boot_services: &crate::BootServices,
        image_handle: *mut (),
        path: &str,
        screen: &mut Screen,
        max_size: usize,
        start_line: usize, // where to render status messages
    ) -> Result<(*mut u8, usize), &'static str> {
        use crate::uefi::file_system::*;

        unsafe {
            let loaded_image = get_loaded_image(boot_services, image_handle)
                .map_err(|_| {
                    morpheus_core::logger::log("FAIL: get_loaded_image");
                    "Failed to get loaded image"
                })?;

            let device_handle = (*loaded_image).device_handle;
            let fs_protocol = get_file_system_protocol(boot_services, device_handle)
                .map_err(|_| {
                    morpheus_core::logger::log("FAIL: get_file_system_protocol");
                    "Failed to get file system protocol"
                })?;
            let root = open_root_volume(fs_protocol).map_err(|_| {
                morpheus_core::logger::log("FAIL: open_root_volume");
                "Failed to open root volume"
            })?;

            screen.put_str_at(5, start_line, "Opening file...", EFI_DARKGREEN, EFI_BLACK);

            let mut utf16_path = [0u16; 256];
            ascii_to_utf16(path, &mut utf16_path);

            let file = open_file_read(root, &utf16_path).map_err(|status| {
                morpheus_core::logger::log("FAIL: open_file_read");
                if status == 0x80000000000000 | 14 {
                    "File not found"
                } else {
                    "Failed to open file"
                }
            })?;

            screen.put_str_at(
                5,
                start_line + 1,
                "Allocating pages...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );

            // Try allocating smaller chunks if big allocation fails
            // Start with max_size, then try 256MB, 128MB, 64MB chunks
            const PAGE_SIZE: usize = 4096;
            let chunk_sizes = [
                max_size,
                256 * 1024 * 1024, // 256MB
                128 * 1024 * 1024, // 128MB
                64 * 1024 * 1024,  // 64MB
            ];

            let mut buffer_addr = 0u64;
            let mut chunk_size = 0usize;
            let alloc_type = 0; // EFI_ALLOCATE_ANY_PAGES - less fragmentation
            let mem_type = 2; // EFI_LOADER_DATA

            // Try progressively smaller allocations
            for &size in &chunk_sizes {
                let pages_needed = (size + PAGE_SIZE - 1) / PAGE_SIZE;
                let mut addr = 0u64; // ANY_PAGES doesn't need initial address
                
                let status = (boot_services.allocate_pages)(
                    alloc_type,
                    mem_type,
                    pages_needed,
                    &mut addr,
                );

                if status == 0 {
                    buffer_addr = addr;
                    chunk_size = size;
                    break;
                }
            }

            if buffer_addr == 0 {
                morpheus_core::logger::log("FAIL: all allocate_pages attempts");
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to allocate UEFI pages");
            }

            let buffer_ptr = buffer_addr as *mut u8;
            let mut bytes_to_read = chunk_size;

            screen.put_str_at(5, start_line + 2, "Reading file...", EFI_DARKGREEN, EFI_BLACK);
            
            // Read up to chunk_size - UEFI will update bytes_to_read with actual amount
            let status = ((*file).read)(file, &mut bytes_to_read, buffer_ptr);

            if status != 0 {
                morpheus_core::logger::log("FAIL: file.read");
                let pages = (chunk_size + PAGE_SIZE - 1) / PAGE_SIZE;
                (boot_services.free_pages)(buffer_addr, pages);
                close_file(file).ok();
                close_file(root).ok();
                return Err("Failed to read file");
            }

            close_file(file).ok();
            close_file(root).ok();

            screen.put_str_at(5, start_line + 3, "Success!", EFI_GREEN, EFI_BLACK);

            // Return pointer + actual bytes read (GRUB style)
            Ok((buffer_ptr, bytes_to_read))
        }
    }
}
