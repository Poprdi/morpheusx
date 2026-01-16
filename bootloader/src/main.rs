//! Morpheus UEFI Bootloader - Bare-metal Platform Entry
//!
//! Minimal UEFI trampoline that immediately enters our bare-metal world.
//! UEFI is only used to:
//! 1. Get GOP framebuffer info
//! 2. Call ExitBootServices
//!
//! Everything after that runs on our platform.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_mut)]

extern crate alloc;

use core::panic::PanicInfo;

mod baremetal;
mod boot;
mod installer;
mod tui;
mod uefi;
mod uefi_allocator;

// These modules still exist but will be ported to post-EBS framebuffer
// For now they're dormant
#[allow(unused_imports)]
use tui::renderer::Screen;

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
        reset_type: u32, // 0=Cold, 1=Warm, 2=Shutdown, 3=PlatformSpecific
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
pub struct BootServices {
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
    // Miscellaneous Services
    _get_next_monotonic_count: usize,
    /// Stall for microseconds
    pub stall: extern "efiapi" fn(microseconds: usize) -> usize,
    /// Disable/set watchdog timer (timeout in seconds, 0 = disable)
    pub set_watchdog_timer: extern "efiapi" fn(
        timeout: usize,
        watchdog_code: u64,
        data_size: usize,
        watchdog_data: *const u16,
    ) -> usize,
    // Driver Support Services
    _connect_controller: usize,
    _disconnect_controller: usize,
    // Open/Close Protocol Services
    pub open_protocol: extern "efiapi" fn(
        handle: *mut (),
        protocol: *const [u8; 16],
        interface: *mut *mut (),
        agent_handle: *mut (),
        controller_handle: *mut (),
        attributes: u32,
    ) -> usize,
    _close_protocol: usize,
    _open_protocol_information: usize,
    // Library Services
    _protocols_per_handle: usize,
    _locate_handle_buffer: usize,
    // Protocol Interface Services (continued)
    pub locate_protocol: extern "efiapi" fn(
        protocol: *const [u8; 16],
        registration: *const (),
        interface: *mut *mut (),
    ) -> usize,
    _install_multiple_protocol_interfaces: usize,
    _uninstall_multiple_protocol_interfaces: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// GRAPHICS OUTPUT PROTOCOL (GOP)
// ═══════════════════════════════════════════════════════════════════════════

/// GOP Protocol GUID: 9042A9DE-23DC-4A38-96FB-7ADED080516A
pub const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: [u8; 16] = [
    0xDE, 0xA9, 0x42, 0x90, 0xDC, 0x23, 0x38, 0x4A, 0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A,
];

/// GOP Pixel Format
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GopPixelFormat {
    /// Red-Green-Blue-Reserved 8-bits per color
    Rgbx = 0,
    /// Blue-Green-Red-Reserved 8-bits per color
    Bgrx = 1,
    /// Pixel format defined by pixel bitmask
    BitMask = 2,
    /// No direct framebuffer access
    BltOnly = 3,
}

/// GOP Mode Information
#[repr(C)]
pub struct GopModeInfo {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: GopPixelFormat,
    pub pixel_information: [u32; 4], // PixelBitmask (only for BitMask format)
    pub pixels_per_scan_line: u32,
}

/// GOP Mode
#[repr(C)]
pub struct GopMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *const GopModeInfo,
    pub size_of_info: usize,
    pub frame_buffer_base: u64,
    pub frame_buffer_size: usize,
}

/// Graphics Output Protocol
#[repr(C)]
pub struct GraphicsOutputProtocol {
    pub query_mode: extern "efiapi" fn(
        this: *mut GraphicsOutputProtocol,
        mode_number: u32,
        size_of_info: *mut usize,
        info: *mut *const GopModeInfo,
    ) -> usize,
    pub set_mode: extern "efiapi" fn(this: *mut GraphicsOutputProtocol, mode_number: u32) -> usize,
    pub blt: usize, // We don't use Blt
    pub mode: *mut GopMode,
}

#[no_mangle]
pub extern "efiapi" fn efi_main(image_handle: *mut (), system_table: *const ()) -> usize {
    unsafe {
        let st = &*(system_table as *const SystemTable);
        let bs = &*st.boot_services;

        // Set boot services for UEFI-backed global allocator (briefly needed for setup)
        uefi_allocator::set_boot_services(st.boot_services);

        // ═══════════════════════════════════════════════════════════════════
        // STEP 1: Get GOP framebuffer info
        // ═══════════════════════════════════════════════════════════════════
        let mut gop_ptr: *mut GraphicsOutputProtocol = core::ptr::null_mut();
        let status = (bs.locate_protocol)(
            &EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
            core::ptr::null(),
            &mut gop_ptr as *mut _ as *mut *mut (),
        );

        let framebuffer_info = if status == 0 && !gop_ptr.is_null() {
            let gop = &*gop_ptr;
            let mode = &*gop.mode;
            let info = &*mode.info;

            baremetal::FramebufferInfo {
                base: mode.frame_buffer_base,
                size: mode.frame_buffer_size,
                width: info.horizontal_resolution,
                height: info.vertical_resolution,
                stride: info.pixels_per_scan_line,
                format: info.pixel_format as u32,
            }
        } else {
            // No GOP - use zeroed info (will need serial-only output)
            baremetal::FramebufferInfo {
                base: 0,
                size: 0,
                width: 0,
                height: 0,
                stride: 0,
                format: 0,
            }
        };

        // ═══════════════════════════════════════════════════════════════════
        // STEP 2: Enter bare-metal world - NEVER RETURNS
        // ═══════════════════════════════════════════════════════════════════
        let config = baremetal::BaremetalEntryConfig {
            image_handle,
            system_table,
            framebuffer: framebuffer_info,
        };

        baremetal::enter_baremetal(config);
        // NEVER REACHED
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // In bare-metal mode, just halt
    // TODO: Could output to serial if available
    loop {
        core::hint::spin_loop();
    }
}
