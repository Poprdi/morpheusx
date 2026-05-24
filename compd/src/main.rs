#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{compositor as compsys, entry, hw, process};

mod font;
mod islands;
mod messages;

entry!(main);

fn main() -> i32 {
    compsys::compositor_set().expect("compositor_set failed — another compositor is registered");

    let fb_info = hw::fb_info().expect("fb_info failed");
    let fb_vaddr = hw::fb_map().expect("fb_map failed");
    let fb_ptr = fb_vaddr as *mut u32;

    // fb_info.stride is bytes; convert to pixels for blit math.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    let mut state =
        islands::CompState::new(fb_ptr, fb_info.width, fb_info.height, fb_stride_px, is_bgrx);

    if let Some(a) = libmorpheus::desktop::DesktopAppearance::load() {
        state.apply_desktop_appearance(&a);
    }

    let mut last_appearance_poll_ms = 0u64;

    loop {
        let now_ms = libmorpheus::time::uptime_ms();
        if now_ms.saturating_sub(last_appearance_poll_ms) >= 400 {
            if let Some(a) = libmorpheus::desktop::DesktopAppearance::load() {
                state.apply_desktop_appearance(&a);
            }
            last_appearance_poll_ms = now_ms;
        }

        islands::vsync::tick(&mut state);
        islands::input::poll(&mut state);
        islands::surface_mgr::update(&mut state);
        islands::focus::process_msgs(&mut state);
        islands::renderer::compose(&mut state);

        process::yield_cpu();
    }
}
