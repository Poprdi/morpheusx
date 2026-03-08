#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{compositor as compsys, entry, hw, process, io};

mod islands;
mod font;

entry!(main);

fn main() -> i32 {
    io::println("shelld: starting");

    // wait for compd to be registered.
    // if this returns u64::MAX, compd isn't up yet. yield and try again like a civilized process.
    loop {
        let mut buf = [compsys::SurfaceEntry {
            pid: 0, _pad: 0, phys_addr: 0, pages: 0,
            width: 0, height: 0, stride: 0, format: 0, dirty: 0, _pad2: 0,
        }; 1];
        let r = compsys::surface_list(&mut buf);
        // surface_list returns usize::MAX when EPERM (compositor not registered)
        if r != usize::MAX {
            break;
        }
        process::yield_cpu();
    }

    io::println("shelld: compositor detected");

    // get framebuffer info — dimensions, stride, format
    let fb_info = hw::fb_info().expect("shelld: fb_info failed");

    // map our private offscreen surface buffer.
    // because compd is the compositor, fb_map gives us a private buffer, not the real FB.
    let surface_vaddr = hw::fb_map().expect("shelld: fb_map failed");
    let surface_ptr = surface_vaddr as *mut u32;

    // stride is bytes not pixels. yes again. yes i know.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    io::println("shelld: surface mapped, entering main loop");

    let mut state = islands::ShellState::new(
        surface_ptr, fb_info.width, fb_info.height, fb_stride_px, is_bgrx,
    );

    loop {
        // render wallpaper (only when dirty — first frame)
        islands::wallpaper::tick(&mut state);

        // render panel (every tick for clock updates)
        islands::panel::tick(&mut state);

        // render launcher overlay (if open)
        islands::launcher::tick(&mut state);

        // poll mouse input forwarded from compd
        poll_mouse(&mut state);

        // tell kernel our surface has new content
        hw::fb_mark_dirty();

        process::yield_cpu();
    }
}

fn poll_mouse(state: &mut islands::ShellState) {
    let ms = hw::mouse_read();
    if ms.dx == 0 && ms.dy == 0 && ms.buttons == 0 {
        return;
    }

    // clamped. because the mouse delta from the kernel is signed and the universe is cruel.
    let fb_w = state.fb_w as i32;
    let fb_h = state.fb_h as i32;
    state.mouse_x = (state.mouse_x + ms.dx as i32).clamp(0, fb_w - 1);
    state.mouse_y = (state.mouse_y + ms.dy as i32).clamp(0, fb_h - 1);

    let left = (ms.buttons & 1) != 0;
    let left_was = (state.last_buttons & 1) != 0;
    let left_pressed = left && !left_was;

    state.last_buttons = ms.buttons;

    if left_pressed {
        islands::launcher::handle_click(state, state.mouse_x, state.mouse_y);
    }
}
