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

    // Override the built-in German layout with the active .kmap on the FS, if
    // present. Re-checked in the loop so layouts hot-swap (e.g. a settings GUI
    // writing /system/keymap.kmap) without a reboot.
    if let Some(km) = load_active_keymap() {
        state.keymap = km;
    }

    // Baseline the taskbar focus-request token so a stale request left in the (cross-boot) persist
    // store can't activate a window before the desktop is even up — only a strictly-new token from
    // this session's shell is serviced (mirrors hypnos baselining the launch request).
    state.focus_req_token = islands::focus::read_focus_request().0;

    let mut last_appearance_poll_ms = 0u64;
    let mut last_keymap_poll_ms = 0u64;

    loop {
        let now_ms = libmorpheus::time::uptime_ms();
        if now_ms.saturating_sub(last_appearance_poll_ms) >= 400 {
            if let Some(a) = libmorpheus::desktop::DesktopAppearance::load() {
                state.apply_desktop_appearance(&a);
            }
            last_appearance_poll_ms = now_ms;
        }

        if now_ms.saturating_sub(last_keymap_poll_ms) >= 1000 {
            if let Some(km) = load_active_keymap() {
                state.keymap = km;
            }
            last_keymap_poll_ms = now_ms;
        }

        islands::vsync::tick(&mut state);
        islands::input::poll(&mut state);
        islands::surface_mgr::update(&mut state);
        // Service taskbar-chip activations (focus/raise · minimize · restore) from the shell, then
        // any keyboard focus-cycle, then publish the resulting focus/minimized snapshot back to the
        // shell so the taskbar chips reflect it.
        islands::focus::consume_focus_request(&mut state);
        islands::focus::process_msgs(&mut state);
        islands::focus::publish_window_state(&mut state);
        islands::renderer::compose(&mut state);

        process::yield_cpu();
    }
}

/// Load the active keyboard layout from `/system/keymap.kmap`. Returns `None`
/// if the file is absent or invalid, in which case compd keeps its current
/// layout (built-in German QWERTZ on first boot).
fn load_active_keymap() -> Option<keymap::Keymap> {
    let data = libmorpheus::fs::read_to_vec("/system/keymap.kmap").ok()?;
    keymap::Keymap::parse(&data)
}
