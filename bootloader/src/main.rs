//! Morpheus UEFI Bootloader - Hello World
//!
//! First UEFI application that displays "Morpheus" on screen.

#![no_std]
#![no_main]

extern crate alloc;

use core::panic::PanicInfo;

mod tui;
mod uefi;
mod installer;
mod boot;

use tui::renderer::Screen;
use tui::logo::{LOGO_LINES_RAW, LOGO_WIDTH, TAGLINE, TAGLINE_WIDTH};
use tui::rain::MatrixRain;
use tui::boot_sequence::BootSequence;
use tui::input::Keyboard;
use tui::main_menu::{MainMenu, MenuAction};
use tui::storage_manager::StorageManager;
use tui::installer_menu::InstallerMenu;
use tui::distro_launcher::DistroLauncher;

#[repr(C)]
pub struct SimpleTextInputProtocol {
    reset: extern "efiapi" fn(*mut SimpleTextInputProtocol, bool) -> usize,
    read_key_stroke: extern "efiapi" fn(*mut SimpleTextInputProtocol, *mut tui::input::InputKey) -> usize,
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
    query_mode: extern "efiapi" fn(*mut SimpleTextOutputProtocol, usize, *mut usize, *mut usize) -> usize,
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
    runtime_services: *const (),
    boot_services: *const BootServices,
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
        memory: u64,
    ) -> usize,
    _free_pages: usize,
    pub get_memory_map: extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut u8,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> usize,
    allocate_pool: extern "efiapi" fn(
        pool_type: usize,
        size: usize,
        buffer: *mut *mut u8,
    ) -> usize,
    free_pool: extern "efiapi" fn(buffer: *mut u8) -> usize,
    // Event & Timer Services
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    // Protocol Handler Services  
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
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
    _locate_device_path: usize,
    _install_configuration_table: usize,
    // Image Services
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,
    pub exit_boot_services: extern "efiapi" fn(
        image_handle: *mut (),
        map_key: usize,
    ) -> usize,
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
                screen.put_str_at(logo_x, y, line, tui::renderer::EFI_GREEN, tui::renderer::EFI_BLACK);
            }
        }
        
        // Draw tagline centered
        let tagline_y = logo_y + LOGO_LINES_RAW.len() + 1;
        let tagline_x = screen.center_x(TAGLINE_WIDTH);
        if tagline_y < screen_height {
            screen.put_str_at(tagline_x, tagline_y, TAGLINE, tui::renderer::EFI_GREEN, tui::renderer::EFI_BLACK);
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
        rain.render_frame(&mut screen);
        
        morpheus_core::logger::log("UEFI system table acquired");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        morpheus_core::logger::log("Console output protocol ready");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        morpheus_core::logger::log("Keyboard input protocol ready");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        // Enumerate storage devices
        let bs = &*system_table.boot_services;
        let mut temp_disk_manager = morpheus_core::disk::manager::DiskManager::new();
        match crate::uefi::disk::enumerate_disks(bs, &mut temp_disk_manager) {
            Ok(()) => {
                let disk_count = temp_disk_manager.disk_count();
                if disk_count > 0 {
                    morpheus_core::logger::log("Block I/O protocol initialized");
                    boot_seq.render(&mut screen, boot_x, boot_y);
                    rain.render_frame(&mut screen);
                    
                    morpheus_core::logger::log("Storage devices enumerated");
                    boot_seq.render(&mut screen, boot_x, boot_y);
                    rain.render_frame(&mut screen);
                } else {
                    morpheus_core::logger::log("No storage devices detected");
                    boot_seq.render(&mut screen, boot_x, boot_y);
                    rain.render_frame(&mut screen);
                }
            }
            Err(_) => {
                morpheus_core::logger::log("Warning: Storage enumeration failed");
                boot_seq.render(&mut screen, boot_x, boot_y);
                rain.render_frame(&mut screen);
            }
        }
        
        morpheus_core::logger::log("TUI renderer initialized");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        morpheus_core::logger::log("Matrix rain effect loaded");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        morpheus_core::logger::log("Main menu system ready");
        boot_seq.render(&mut screen, boot_x, boot_y);
        rain.render_frame(&mut screen);
        
        boot_seq.mark_complete();
        boot_seq.render(&mut screen, boot_x, boot_y);
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
                    let mut launcher = DistroLauncher::new();
                    launcher.run(&mut screen, &mut keyboard, bs, st_ptr, image_handle);
                    // Returns when user presses ESC, loop continues to main menu
                }
                MenuAction::DistroDownloader => {
                    screen.clear();
                    screen.put_str_at(5, 10, "Distro Downloader - Coming soon...", tui::renderer::EFI_LIGHTGREEN, tui::renderer::EFI_BLACK);
                    keyboard.wait_for_key();
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
                    screen.put_str_at(5, 10, "Admin Functions - Coming soon...", tui::renderer::EFI_LIGHTGREEN, tui::renderer::EFI_BLACK);
                    keyboard.wait_for_key();
                }
                MenuAction::ExitToFirmware => {
                    screen.clear();
                    screen.put_str_at(5, 10, "Exiting to firmware...", tui::renderer::EFI_LIGHTGREEN, tui::renderer::EFI_BLACK);
                    break; // Exit the loop
                }
                _ => {}
            }
        }
        
        // Final rain loop
        loop {
            rain.render_frame(&mut screen);
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
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
