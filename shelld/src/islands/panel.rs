use crate::islands::{draw_text, raw_fill, ShellState, PANEL_H, START_BTN_W};
use libmorpheus::time;

/// panel island. taskbar at the bottom of the screen.
/// renders START button, status text, and an uptime clock.
pub fn tick(state: &mut ShellState) {
    if !state.panel_dirty {
        return;
    }

    let panel_y = state.fb_h.saturating_sub(PANEL_H);
    let (pr, pg, pb) = state.panel_rgb;
    let panel_bg = state.pack(pr, pg, pb);

    // panel background bar
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        0,
        panel_y,
        state.fb_w,
        PANEL_H,
        panel_bg,
    );

    // START button
    let (sr, sg, sb) = if state.launcher_open { state.start_active_rgb } else { state.start_rgb };
    let start_bg = state.pack(sr, sg, sb);
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        0,
        panel_y,
        START_BTN_W,
        PANEL_H,
        start_bg,
    );
    let start_fg = (255u8, 255u8, 255u8);
    let start_bg_rgb = if state.launcher_open { state.start_active_rgb } else { state.start_rgb };
    draw_text(state, 10, panel_y + 7, "START", start_fg, start_bg_rgb);

    // status label
    draw_text(
        state,
        START_BTN_W + 12,
        panel_y + 7,
        "MorpheusX DE",
        (200, 200, 210),
        state.panel_rgb,
    );

    // uptime clock on the right side
    let ns = time::clock_gettime();
    let secs = (ns / 1_000_000_000) as u32;
    let mins = secs / 60;
    let hours = mins / 60;
    let mut clock_buf = [0u8; 16];
    let clock_len = format_clock(&mut clock_buf, hours % 100, mins % 60, secs % 60);
    if let Ok(clock_str) = core::str::from_utf8(&clock_buf[..clock_len]) {
        let text_w = clock_len as u32 * 8;
        let clock_x = state.fb_w.saturating_sub(text_w + 12);
        draw_text(
            state,
            clock_x,
            panel_y + 7,
            clock_str,
            (180, 220, 180),
            state.panel_rgb,
        );
    }

    // panel repaints every tick for the clock
    state.panel_dirty = true;
}

// format HH:MM:SS into a fixed buffer. no alloc. no format!. just digits.
fn format_clock(buf: &mut [u8; 16], h: u32, m: u32, s: u32) -> usize {
    buf[0] = b'0' + (h / 10) as u8;
    buf[1] = b'0' + (h % 10) as u8;
    buf[2] = b':';
    buf[3] = b'0' + (m / 10) as u8;
    buf[4] = b'0' + (m % 10) as u8;
    buf[5] = b':';
    buf[6] = b'0' + (s / 10) as u8;
    buf[7] = b'0' + (s % 10) as u8;
    8
}
