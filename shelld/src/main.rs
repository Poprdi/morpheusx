#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{compositor as compsys, entry, hw, info, process};

mod font;
mod islands;

entry!(main);

fn main() -> i32 {
    info!("starting");

    // surface_list returns usize::MAX until compd registers.
    loop {
        let mut buf = [compsys::SurfaceEntry {
            pid: 0,
            _pad: 0,
            phys_addr: 0,
            pages: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: 0,
            dirty: 0,
            _pad2: 0,
        }; 1];
        let r = compsys::surface_list(&mut buf);
        if r != usize::MAX {
            break;
        }
        process::yield_cpu();
    }

    info!("compositor detected");

    let fb_info = hw::fb_info().expect("shelld: fb_info failed");

    // compd owns the compositor slot; fb_map returns a private offscreen buffer.
    let surface_vaddr = hw::fb_map().expect("shelld: fb_map failed");
    let surface_ptr = surface_vaddr as *mut u32;

    // fb_info.stride is bytes.
    let fb_stride_px = fb_info.stride / 4;
    let is_bgrx = fb_info.format == 1;

    info!("surface mapped, entering main loop");

    let mut state = islands::ShellState::new(
        surface_ptr,
        fb_info.width,
        fb_info.height,
        fb_stride_px,
        is_bgrx,
    );

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

        islands::wallpaper::tick(&mut state);
        islands::panel::tick(&mut state);
        islands::launcher::tick(&mut state);

        poll_mouse(&mut state);

        hw::fb_mark_dirty();

        process::yield_cpu();
    }
}

fn poll_mouse(state: &mut islands::ShellState) {
    let ms = hw::mouse_read();
    if ms.dx == 0 && ms.dy == 0 && ms.buttons == 0 {
        return;
    }

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
