#![no_std]
#![no_main]
#![allow(dead_code)]

extern crate alloc;

mod chambers;
mod font;
mod layout;
mod state;
mod theme;
mod widgets;

use libmorpheus::{entry, hw, io, process};

entry!(main);

fn main() -> i32 {
    io::println("settings: starting");

    let fb_info = hw::fb_info().expect("settings: fb_info failed");
    let surface_vaddr = hw::fb_map().expect("settings: fb_map failed");
    let surface_ptr = surface_vaddr as *mut u32;

    // stride is bytes not pixels. yes again.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    let mut app = state::SettingsApp::new(
        surface_ptr,
        fb_info.width,
        fb_info.height,
        fb_stride_px,
        is_bgrx,
    );

    app.init();

    loop {
        app.tick();
        hw::fb_mark_dirty();
        process::yield_cpu();
    }
}
