#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{compositor as compsys, entry, hw, process};

mod messages;
mod islands;
mod font;

entry!(main);

fn main() -> i32 {
    // 1. register as compositor. panics if another process holds the slot.
    compsys::compositor_set().expect("compositor_set failed — another compositor is registered");

    // 2. get framebuffer info
    let fb_info = hw::fb_info().expect("fb_info failed");

    // 3. map the physical framebuffer
    let fb_vaddr = hw::fb_map().expect("fb_map failed");
    let fb_ptr = fb_vaddr as *mut u32;

    // stride is bytes not pixels. yes again. yes i know.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    // 4. initialize island state
    let mut state = islands::CompState::new(fb_ptr, fb_info.width, fb_info.height,
                                             fb_stride_px, is_bgrx);

    // 5. enter main vsync loop
    loop {
        islands::vsync::tick(&mut state);
        islands::input::poll(&mut state);
        islands::surface_mgr::update(&mut state);
        islands::focus::process_msgs(&mut state);

        // compose if any surface is mapped (or always, to show desktop)
        islands::renderer::compose(&mut state);

        process::yield_cpu();
    }
}
