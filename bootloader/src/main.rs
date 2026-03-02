//! Morpheus — UEFI trampoline.
//!
//! Queries GOP, hands off to baremetal::enter_baremetal(). That's it.
//! The only UEFI interaction in the entire system happens here and in
//! enter_baremetal's EBS call. Everything after is ours.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::panic::PanicInfo;

mod baremetal;
mod bsod;
mod storage;
mod tui;
mod uefi_allocator;

// UEFI FFI — minimal subset: just enough to locate GOP and feed the allocator

/// Only field we read is boot_services. Everything else is padding.
#[repr(C)]
struct SystemTable {
    _header: [u8; 24],
    _firmware_vendor: *const u16,
    _firmware_revision: u32,
    _console_in_handle: *const (),
    _con_in: *mut (),
    _console_out_handle: *const (),
    _con_out: *mut (),
    _stderr_handle: *const (),
    _stderr: *const (),
    _runtime_services: *const (),
    boot_services: *const BootServices,
}

/// Pre-EBS allocator uses allocate_pool/free_pool.
/// efi_main uses locate_protocol for GOP.
/// Layout must match UEFI spec offset-for-offset up through locate_protocol.
#[repr(C)]
pub struct BootServices {
    _header: [u8; 24],
    _raise_tpl: usize,
    _restore_tpl: usize,
    _allocate_pages: usize,
    _free_pages: usize,
    _get_memory_map: usize,
    pub allocate_pool: extern "efiapi" fn(usize, usize, *mut *mut u8) -> usize,
    pub free_pool: extern "efiapi" fn(*mut u8) -> usize,
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    _handle_protocol: usize,
    _reserved: usize,
    _register_protocol_notify: usize,
    _locate_handle: usize,
    _locate_device_path: usize,
    _install_configuration_table: usize,
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,
    _exit_boot_services: usize,
    _get_next_monotonic_count: usize,
    _stall: usize,
    _set_watchdog_timer: usize,
    _connect_controller: usize,
    _disconnect_controller: usize,
    _open_protocol: usize,
    _close_protocol: usize,
    _open_protocol_information: usize,
    _protocols_per_handle: usize,
    _locate_handle_buffer: usize,
    pub locate_protocol: extern "efiapi" fn(*const [u8; 16], *const (), *mut *mut ()) -> usize,
}

// GOP — just enough to read the framebuffer address

const GOP_GUID: [u8; 16] = [
    0xDE, 0xA9, 0x42, 0x90, 0xDC, 0x23, 0x38, 0x4A, 0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A,
];

#[repr(C)]
struct GopModeInfo {
    _version: u32,
    horizontal_resolution: u32,
    vertical_resolution: u32,
    pixel_format: u32,
    _pixel_bitmask: [u32; 4],
    pixels_per_scan_line: u32,
}

#[repr(C)]
struct GopMode {
    _max_mode: u32,
    _mode: u32,
    info: *const GopModeInfo,
    _size_of_info: usize,
    frame_buffer_base: u64,
    frame_buffer_size: usize,
}

#[repr(C)]
struct Gop {
    _query_mode: usize,
    _set_mode: usize,
    _blt: usize,
    mode: *mut GopMode,
}

// ENTRY

#[no_mangle]
pub extern "efiapi" fn efi_main(image_handle: *mut (), system_table: *const ()) -> usize {
    unsafe {
        // Raw COM1 write — needs zero init, works from first instruction.
        // If you see this on serial, OVMF found and loaded our binary.
        morpheus_hwinit::serial::puts("[MORPHEUSX] efi_main\n");

        let st = &*(system_table as *const SystemTable);
        let bs = &*st.boot_services;

        // Pre-EBS allocator needs BootServices for allocate_pool/free_pool
        uefi_allocator::set_boot_services(st.boot_services);

        // Query GOP for framebuffer info
        let mut gop_ptr: *mut Gop = core::ptr::null_mut();
        let status = (bs.locate_protocol)(
            &GOP_GUID,
            core::ptr::null(),
            &mut gop_ptr as *mut _ as *mut *mut (),
        );

        let fb = if status == 0 && !gop_ptr.is_null() {
            let mode = &*(*gop_ptr).mode;
            let info = &*mode.info;
            baremetal::FramebufferInfo {
                base: mode.frame_buffer_base,
                size: mode.frame_buffer_size,
                width: info.horizontal_resolution,
                height: info.vertical_resolution,
                stride: info.pixels_per_scan_line,
                format: info.pixel_format,
            }
        } else {
            baremetal::FramebufferInfo {
                base: 0,
                size: 0,
                width: 0,
                height: 0,
                stride: 0,
                format: 0,
            }
        };

        // Cross the border. Never returns.
        baremetal::enter_baremetal(baremetal::BaremetalEntryConfig {
            image_handle,
            system_table,
            framebuffer: fb,
        })
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Emit as much as possible to serial. Never use alloc here (may be OOM).
    morpheus_hwinit::serial::puts("\n[PANIC] ");
    if let Some(loc) = info.location() {
        morpheus_hwinit::serial::puts(loc.file());
        morpheus_hwinit::serial::puts(":");
        // Print line number as decimal digits (no alloc needed).
        let line = loc.line();
        let mut digits = [0u8; 10];
        let mut n = line;
        let mut len = 0usize;
        if n == 0 {
            digits[0] = b'0';
            len = 1;
        } else {
            while n > 0 {
                digits[len] = b'0' + (n % 10) as u8;
                len += 1;
                n /= 10;
            }
            digits[..len].reverse();
        }
        if let Ok(s) = core::str::from_utf8(&digits[..len]) {
            morpheus_hwinit::serial::puts(s);
        }
    }
    morpheus_hwinit::serial::puts(" — PANIC (spinning)\n");

    // Show BSoD panic screen on the framebuffer.
    if let Some(loc) = info.location() {
        unsafe {
            bsod::show_panic_screen(loc.file(), loc.line(), loc.column());
        }
    } else {
        unsafe {
            bsod::show_panic_screen("<unknown>", 0, 0);
        }
    }

    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
