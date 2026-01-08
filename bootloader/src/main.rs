//! Morpheus UEFI Bootloader - Hello World
//!
//! First UEFI application that displays "Morpheus" on screen.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_mut)]

extern crate alloc;

use core::panic::PanicInfo;

mod boot;
mod installer;
mod tui;
mod uefi;

use tui::boot_sequence::{BootSequence, NetworkBootResult};
use tui::distro_launcher::DistroLauncher;
use tui::input::Keyboard;
use tui::installer_menu::InstallerMenu;
use tui::logo::{LOGO_LINES_RAW, LOGO_WIDTH, TAGLINE, TAGLINE_WIDTH};
use tui::main_menu::{MainMenu, MenuAction};
use tui::rain::MatrixRain;
use tui::renderer::Screen;
use tui::storage_manager::StorageManager;

#[repr(C)]
pub struct SimpleTextInputProtocol {
    reset: extern "efiapi" fn(*mut SimpleTextInputProtocol, bool) -> usize,
    read_key_stroke:
        extern "efiapi" fn(*mut SimpleTextInputProtocol, *mut tui::input::InputKey) -> usize,
}

#[repr(C)]
pub struct SimpleTextOutputMode {
    max_mode: i32,
    mode: i32,
    attribute: i32,
    cursor_column: i32,
    cursor_row: i32,
    cursor_visible: bool,
}

#[repr(C)]
pub struct SimpleTextOutputProtocol {
    reset: extern "efiapi" fn(*mut SimpleTextOutputProtocol, bool) -> usize,
    output_string: extern "efiapi" fn(*mut SimpleTextOutputProtocol, *const u16) -> usize,
    test_string: usize,
    query_mode:
        extern "efiapi" fn(*mut SimpleTextOutputProtocol, usize, *mut usize, *mut usize) -> usize,
    set_mode: usize,
    set_attribute: extern "efiapi" fn(*mut SimpleTextOutputProtocol, usize) -> usize,
    clear_screen: extern "efiapi" fn(*mut SimpleTextOutputProtocol) -> usize,
    set_cursor_position: extern "efiapi" fn(*mut SimpleTextOutputProtocol, usize, usize) -> usize,
    enable_cursor: extern "efiapi" fn(*mut SimpleTextOutputProtocol, bool) -> usize,
    mode: *const SimpleTextOutputMode,
}

#[repr(C)]
struct SystemTable {
    _header: [u8; 24],
    _firmware_vendor: *const u16,
    _firmware_revision: u32,
    _console_in_handle: *const (),
    con_in: *mut SimpleTextInputProtocol,
    _console_out_handle: *const (),
    con_out: *mut SimpleTextOutputProtocol,
    _stderr_handle: *const (),
    _stderr: *const (),
    runtime_services: *const RuntimeServices,
    boot_services: *const BootServices,
    number_of_table_entries: usize,
    configuration_table: *const ConfigurationTable,
}

#[repr(C)]
struct RuntimeServices {
    _header: [u8; 24],
    // Time Services
    _get_time: usize,
    _set_time: usize,
    _get_wakeup_time: usize,
    _set_wakeup_time: usize,
    // Virtual Memory Services
    _set_virtual_address_map: usize,
    _convert_pointer: usize,
    // Variable Services
    _get_variable: usize,
    _get_next_variable_name: usize,
    _set_variable: usize,
    // Miscellaneous Services
    _get_next_high_monotonic_count: usize,
    pub reset_system: extern "efiapi" fn(
        reset_type: u32,  // 0=Cold, 1=Warm, 2=Shutdown, 3=PlatformSpecific
        reset_status: usize,
        data_size: usize,
        reset_data: *const (),
    ) -> !,
}

#[repr(C)]
struct ConfigurationTable {
    vendor_guid: [u8; 16],
    vendor_table: *const (),
}

#[repr(C)]
struct BootServices {
    _header: [u8; 24],
    // Task Priority Services
    _raise_tpl: usize,
    _restore_tpl: usize,
    // Memory Services (correct order per UEFI spec)
    pub allocate_pages: extern "efiapi" fn(
        allocate_type: usize,
        memory_type: usize,
        pages: usize,
        memory: *mut u64,
    ) -> usize,
    pub free_pages: extern "efiapi" fn(memory: u64, pages: usize) -> usize,
    pub get_memory_map: extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut u8,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> usize,
    allocate_pool: extern "efiapi" fn(pool_type: usize, size: usize, buffer: *mut *mut u8) -> usize,
    free_pool: extern "efiapi" fn(buffer: *mut u8) -> usize,
    // Event & Timer Services
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    // Protocol Handler Services
    install_protocol_interface: extern "efiapi" fn(
        handle: *mut *mut (),
        protocol: *const [u8; 16],
        interface_type: usize,
        interface: *mut core::ffi::c_void,
    ) -> usize,
    _reinstall_protocol_interface: extern "efiapi" fn(
        handle: *mut (),
        protocol: *const [u8; 16],
        interface_type: usize,
        old_interface: *mut core::ffi::c_void,
        new_interface: *mut core::ffi::c_void,
    ) -> usize,
    uninstall_protocol_interface: extern "efiapi" fn(
        handle: *mut (),
        protocol: *const [u8; 16],
        interface: *mut core::ffi::c_void,
    ) -> usize,
    handle_protocol: extern "efiapi" fn(
        handle: *mut (),
        protocol: *const [u8; 16],
        interface: *mut *mut (),
    ) -> usize,
    _reserved: usize,
    _register_protocol_notify: usize,
    locate_handle: extern "efiapi" fn(
        search_type: usize,
        protocol: *const [u8; 16],
        search_key: *const (),
        buffer_size: *mut usize,
        buffer: *mut *mut (),
    ) -> usize,
    locate_device_path: extern "efiapi" fn(
        protocol: *const [u8; 16],
        device_path: *mut *mut (),
        handle: *mut *mut (),
    ) -> usize,
    install_configuration_table:
        extern "efiapi" fn(guid: *const [u8; 16], table: *const core::ffi::c_void) -> usize,
    // Image Services
    pub load_image: extern "efiapi" fn(
        boot_policy: bool,
        parent_image_handle: *mut (),
        file_path: *const (),
        source_buffer: *const core::ffi::c_void,
        source_size: usize,
        image_handle: *mut *mut (),
    ) -> usize,
    pub start_image: extern "efiapi" fn(
        image_handle: *mut (),
        exit_data_size: *mut usize,
        exit_data: *mut *mut u16,
    ) -> usize,
    _exit: extern "efiapi" fn(*mut (), usize, *const u16) -> usize,
    pub unload_image: extern "efiapi" fn(image_handle: *mut ()) -> usize,
    pub exit_boot_services: extern "efiapi" fn(image_handle: *mut (), map_key: usize) -> usize,
}

#[no_mangle]
pub extern "efiapi" fn efi_main(image_handle: *mut (), system_table: *const ()) -> usize {
    unsafe {
        let system_table = &*(system_table as *const SystemTable);

        // Set global boot services pointer for allocator
        BOOT_SERVICES_PTR = system_table.boot_services;

        let mut screen = Screen::new(system_table.con_out);
        let mut keyboard = Keyboard::new(system_table.con_in);

        screen.clear();

        // Calculate centered positions
        let screen_width = screen.width();
        let screen_height = screen.height();

        // Center logo vertically - put in upper third
        let logo_y = 2;

        // Draw logo centered horizontally
        let logo_x = screen.center_x(LOGO_WIDTH);

        for (i, line) in LOGO_LINES_RAW.iter().enumerate() {
            let y = logo_y + i;
            if y < screen_height {
                screen.put_str_at(
                    logo_x,
                    y,
                    line,
                    tui::renderer::EFI_GREEN,
                    tui::renderer::EFI_BLACK,
                );
            }
        }

        // Draw tagline centered
        let tagline_y = logo_y + LOGO_LINES_RAW.len() + 1;
        let tagline_x = screen.center_x(TAGLINE_WIDTH);
        if tagline_y < screen_height {
            screen.put_str_at(
                tagline_x,
                tagline_y,
                TAGLINE,
                tui::renderer::EFI_GREEN,
                tui::renderer::EFI_BLACK,
            );
        }

        // Boot sequence - log real initialization steps
        let boot_y = tagline_y + 3;
        let boot_x = 5;
        let mut boot_seq = BootSequence::new();

        // Initialize matrix rain
        let mut rain = MatrixRain::new(screen_width, screen_height);

        // Perform actual initialization and log each step
        morpheus_core::logger::log("Morpheus bootloader initialized");
        boot_seq.render(&mut screen, boot_x, boot_y);

        morpheus_core::logger::log("UEFI system table acquired");
        boot_seq.render(&mut screen, boot_x, boot_y);

        morpheus_core::logger::log("Console output protocol ready");
        boot_seq.render(&mut screen, boot_x, boot_y);

        morpheus_core::logger::log("Keyboard input protocol ready");
        boot_seq.render(&mut screen, boot_x, boot_y);

        // Enumerate storage devices
        let bs = &*system_table.boot_services;
        let mut temp_disk_manager = morpheus_core::disk::manager::DiskManager::new();
        match crate::uefi::disk::enumerate_disks(bs, &mut temp_disk_manager) {
            Ok(()) => {
                let disk_count = temp_disk_manager.disk_count();
                if disk_count > 0 {
                    morpheus_core::logger::log("Block I/O protocol initialized");
                    boot_seq.render(&mut screen, boot_x, boot_y);

                    morpheus_core::logger::log("Storage devices enumerated");
                    boot_seq.render(&mut screen, boot_x, boot_y);
                } else {
                    morpheus_core::logger::log("No storage devices detected");
                    boot_seq.render(&mut screen, boot_x, boot_y);
                }
            }
            Err(_) => {
                morpheus_core::logger::log("Warning: Storage enumeration failed");
                boot_seq.render(&mut screen, boot_x, boot_y);
            }
        }

        morpheus_core::logger::log("TUI renderer initialized");
        boot_seq.render(&mut screen, boot_x, boot_y);

        morpheus_core::logger::log("Matrix rain effect loaded");
        boot_seq.render(&mut screen, boot_x, boot_y);

        morpheus_core::logger::log("Main menu system ready");
        boot_seq.render(&mut screen, boot_x, boot_y);

        // Initialize network stack
        // Time function using TSC (timestamp counter)
        fn get_time_ms() -> u64 {
            // SAFETY: Reading TSC is always safe on x86_64
            let tsc = unsafe { morpheus_network::read_tsc() };
            // Approximate conversion - 2GHz assumed (TODO: calibrate properly)
            tsc / 2_000_000
        }

        let _network_result = boot_seq.init_network(&mut screen, boot_x, boot_y, get_time_ms);
        // Network result stored for later use by distro downloader
        // TODO: Store in global or pass to menu system

        boot_seq.mark_complete();
        boot_seq.render(&mut screen, boot_x, boot_y);
        
        // Render rain one final time for visual effect before waiting
        rain.render_frame(&mut screen);

        // Wait for keypress
        keyboard.wait_for_key();

        // Main application loop
        loop {
            // Launch main menu
            let mut main_menu = MainMenu::new(&screen);
            let action = main_menu.run(&mut screen, &mut keyboard);

            // Handle menu action
            match action {
                MenuAction::DistroLauncher => {
                    let bs = &*system_table.boot_services;
                    let st_ptr = system_table as *const SystemTable as *mut ();
                    let mut launcher = DistroLauncher::new(bs, image_handle);
                    launcher.run(&mut screen, &mut keyboard, bs, st_ptr, image_handle);
                }
                MenuAction::DistroDownloader => {
                    let bs = &*system_table.boot_services;
                    // Get disk info for ISO storage
                    // ESP typically starts after GPT headers, disk size from first disk
                    let (esp_lba, disk_lba) = {
                        let mut dm = morpheus_core::disk::manager::DiskManager::new();
                        if crate::uefi::disk::enumerate_disks(bs, &mut dm).is_ok() && dm.disk_count() > 0 {
                            if let Some(disk) = dm.get_disk(0) {
                                // ESP usually at LBA 2048, use full disk size (last_block + 1)
                                (2048, disk.last_block + 1)
                            } else {
                                (2048, 100_000_000) // ~50GB default
                            }
                        } else {
                            (2048, 100_000_000)
                        }
                    };
                    let mut downloader = tui::distro_downloader::DistroDownloader::new(
                        bs,
                        image_handle,
                        esp_lba,
                        disk_lba,
                    );
                    // Downloader manages ISOs (download/delete), boot happens via DistroLauncher
                    downloader.run(&mut screen, &mut keyboard);
                }
                MenuAction::StorageManager => {
                    let bs = &*system_table.boot_services;
                    let mut storage_mgr = StorageManager::new(&screen);
                    storage_mgr.run(&mut screen, &mut keyboard, bs);
                    // Returns when user presses ESC, loop continues to main menu
                }
                MenuAction::SystemSettings => {
                    let bs = &*system_table.boot_services;
                    let mut installer_menu = InstallerMenu::new(image_handle);
                    installer_menu.run(&mut screen, &mut keyboard, bs);
                    // Returns when user presses ESC, loop continues to main menu
                }
                MenuAction::AdminFunctions => {
                    screen.clear();
                    screen.put_str_at(
                        5,
                        10,
                        "Admin Functions - Coming soon...",
                        tui::renderer::EFI_LIGHTGREEN,
                        tui::renderer::EFI_BLACK,
                    );
                    keyboard.wait_for_key();
                }
                MenuAction::ExitToFirmware => {
                    screen.clear();
                    screen.put_str_at(
                        5,
                        10,
                        "Exiting to firmware...",
                        tui::renderer::EFI_LIGHTGREEN,
                        tui::renderer::EFI_BLACK,
                    );
                    
                    // Actually exit to firmware using UEFI ResetSystem
                    unsafe {
                        let runtime_services = &*system_table.runtime_services;
                        // ResetType: 0 = EfiResetCold, 1 = EfiResetWarm, 2 = EfiResetShutdown
                        // Use EfiResetWarm (1) to return to firmware setup
                        (runtime_services.reset_system)(1, 0, 0, core::ptr::null());
                    }
                }
                _ => {}
            }
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Try to display panic information on screen
    // This is best-effort since we may be in a bad state
    unsafe {
        if !BOOT_SERVICES_PTR.is_null() {
            // Try to get console output and display panic
            // We can't use the Screen abstraction here since we might be in a bad state
            // Just spin - at minimum don't silently hang
        }
    }
    
    // Log the panic message if possible
    if let Some(location) = info.location() {
        // We can't allocate in panic handler, so just use static message
        morpheus_core::logger::log("PANIC occurred!");
    } else {
        morpheus_core::logger::log("PANIC occurred (no location)!");
    }
    
    // Infinite loop - system is in bad state
    // TODO: Could trigger UEFI reset after timeout
    loop {
        // Prevent optimization from removing the loop
        core::hint::spin_loop();
    }
}

// UEFI allocator using boot services
use core::alloc::{GlobalAlloc, Layout};

struct UefiAllocator;

static mut BOOT_SERVICES_PTR: *const BootServices = core::ptr::null();

unsafe impl GlobalAlloc for UefiAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let bs = BOOT_SERVICES_PTR;
        if bs.is_null() {
            return core::ptr::null_mut();
        }

        let bs = &*bs;
        let mut buffer: *mut u8 = core::ptr::null_mut();

        // EfiBootServicesData = 2
        let status = (bs.allocate_pool)(2, layout.size(), &mut buffer);

        if status == 0 {
            tui::debug::track_allocation(layout.size());
            buffer
        } else {
            core::ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let bs = BOOT_SERVICES_PTR;
        if bs.is_null() || ptr.is_null() {
            return;
        }

        let bs = &*bs;
        let _ = (bs.free_pool)(ptr);
        tui::debug::track_free(layout.size());
    }
}

#[global_allocator]
static ALLOCATOR: UefiAllocator = UefiAllocator;
