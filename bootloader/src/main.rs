//! Morpheus bootloader binary crate.
//!
//! The authoritative boot sequence lives in `boot.rs` — this file is a
//! thin shell that:
//!
//! 1. Declares the helper modules.
//! 2. Re-exports the UEFI FFI types the global allocator depends on.
//! 3. Provides the language-required `#[panic_handler]`.
//!
//! `efi_main` itself is defined in `boot.rs` with `#[no_mangle]` so the
//! UEFI linker picks it up directly — no indirection through main.

#![no_std]
#![no_main]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use core::panic::PanicInfo;

mod alloc_heap;
mod baremetal_ops;
mod boot;
mod bsod;
mod storage;
mod tui;
mod uefi_allocator;

//
// The hybrid allocator (`uefi_allocator`) needs to call `allocate_pool`
// and `free_pool` while UEFI is still alive. Both this module and
// `boot::EfiBootServices` type-pun the same UEFI spec layout, but the
// allocator was written against the type defined here, so we keep the
// shape (with just the two fields the allocator touches publicly).

/// UEFI BootServices subset, exposed for `uefi_allocator`.
///
/// The layout up to `locate_protocol` must match the UEFI specification
/// byte-for-byte. Fields not used by the allocator are left as `usize`
/// padding.
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

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Emit as much as possible to serial. Never use alloc here (may be OOM).
    morpheus_hal_x86_64::serial::puts("\n[PANIC] ");
    if let Some(loc) = info.location() {
        morpheus_hal_x86_64::serial::puts(loc.file());
        morpheus_hal_x86_64::serial::puts(":");
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
            morpheus_hal_x86_64::serial::puts(s);
        }
    }
    morpheus_hal_x86_64::serial::puts(" — PANIC (spinning)\n");

    // Show BSoD panic screen on the framebuffer (uses boot::published_framebuffer()).
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
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}
