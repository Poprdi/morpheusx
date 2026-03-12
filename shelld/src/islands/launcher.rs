use crate::islands::{draw_text, raw_fill, ShellState, ICON_SIZE, LAUNCHER_H, LAUNCHER_W, PANEL_H};
use libmorpheus::{io, process};

/// launcher island. application launcher overlay triggered by clicking START.
pub fn tick(state: &mut ShellState) {
    if !state.launcher_open || !state.launcher_dirty {
        return;
    }

    let lx = 8u32;
    let ly = state.fb_h.saturating_sub(PANEL_H + LAUNCHER_H + 8);

    // launcher background
    let (lr, lg, lb) = state.launcher_bg_rgb;
    let launcher_bg = state.pack(lr, lg, lb);
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        lx,
        ly,
        LAUNCHER_W,
        LAUNCHER_H,
        launcher_bg,
    );

    // launcher title bar
    let (sr, sg, sb) = state.start_rgb;
    let title_bg = state.pack(sr, sg, sb);
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        lx,
        ly,
        LAUNCHER_W,
        24,
        title_bg,
    );
    draw_text(
        state,
        lx + 8,
        ly + 4,
        "Launcher",
        (255, 255, 255),
        state.start_rgb,
    );

    // shell icon
    let icon_x = lx + 16;
    let icon_y = ly + 40;
    let (ir, ig, ib) = state.icon_bg_rgb;
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        icon_x,
        icon_y,
        ICON_SIZE,
        ICON_SIZE,
        state.pack(ir, ig, ib),
    );
    let (iir, iig, iib) = state.icon_inner_rgb;
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        icon_x + 8,
        icon_y + 10,
        ICON_SIZE - 16,
        ICON_SIZE - 20,
        state.pack(iir, iig, iib),
    );
    draw_text(
        state,
        icon_x + 17,
        icon_y + 20,
        ">_",
        (255, 255, 255),
        state.icon_inner_rgb,
    );
    draw_text(
        state,
        icon_x,
        icon_y + ICON_SIZE + 8,
        "Shell",
        (230, 230, 230),
        state.launcher_bg_rgb,
    );

    state.launcher_dirty = false;
}

/// handle a click at (mx, my) in shell surface coordinates.
/// returns true if the click was consumed by the launcher.
pub fn handle_click(state: &mut ShellState, mx: i32, my: i32) -> bool {
    let panel_y = state.fb_h.saturating_sub(PANEL_H) as i32;

    // check START button click
    if my >= panel_y
        && my < panel_y + PANEL_H as i32
        && mx >= 0
        && mx < crate::islands::START_BTN_W as i32
    {
        state.launcher_open = !state.launcher_open;
        state.launcher_dirty = true;
        // if closing launcher, repaint wallpaper over where it was
        if !state.launcher_open {
            state.wallpaper_dirty = true;
        }
        state.panel_dirty = true;
        return true;
    }

    // check launcher icon click (only if open)
    if state.launcher_open {
        let lx = 8i32;
        let ly = (state.fb_h.saturating_sub(PANEL_H + LAUNCHER_H + 8)) as i32;
        let icon_x = lx + 16;
        let icon_y = ly + 40;

        if mx >= icon_x
            && mx < icon_x + ICON_SIZE as i32
            && my >= icon_y
            && my < icon_y + ICON_SIZE as i32 + 24
        {
            // spawn shell
            match process::spawn("/bin/msh") {
                Ok(_pid) => {
                    io::println("shelld: spawned msh");
                }
                Err(e) => {
                    libmorpheus::println!("shelld: failed to spawn msh err=0x{:x}", e);
                }
            }
            // close launcher after spawning
            state.launcher_open = false;
            state.launcher_dirty = false;
            state.wallpaper_dirty = true;
            state.panel_dirty = true;
            return true;
        }

        // click inside launcher but not on icon — consume it
        if mx >= lx && mx < lx + LAUNCHER_W as i32 && my >= ly && my < ly + LAUNCHER_H as i32 {
            return true;
        }

        // click outside launcher — close it
        state.launcher_open = false;
        state.launcher_dirty = false;
        state.wallpaper_dirty = true;
        state.panel_dirty = true;
        return true;
    }

    false
}
