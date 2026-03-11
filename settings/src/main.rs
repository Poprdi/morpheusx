#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

use alloc::vec;

mod chambers;
mod font;
mod layout;
mod state;
mod theme;
mod widgets;

use libmorpheus::{entry, hw, io};

entry!(main);

fn main() -> i32 {
    io::println("settings: starting");

    let fb_info = hw::fb_info().expect("settings: fb_info failed");
    let surface_vaddr = hw::fb_map().expect("settings: fb_map failed");
    let mapped_surface_ptr = surface_vaddr as *mut u32;

    // stride is bytes not pixels. yes again.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    // render to private software buffer, then publish one memcpy per frame.
    let mut backbuf = vec![0u32; (fb_stride_px as usize).saturating_mul(fb_info.height as usize)];

    let mut app = state::SettingsApp::new(
        backbuf.as_mut_ptr(),
        fb_info.width,
        fb_info.height,
        fb_stride_px,
        is_bgrx,
    );

    app.init();

    loop {
        app.tick();
        unsafe {
            core::ptr::copy_nonoverlapping(
                backbuf.as_ptr(),
                mapped_surface_ptr,
                (fb_stride_px as usize).saturating_mul(fb_info.height as usize),
            );
        }
        let _ = hw::fb_present();
    }
}
