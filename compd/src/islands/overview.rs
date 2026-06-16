//! Overview (Exposé) — the "show all windows" grid. compd's consumer of `wm_geom::overview_*`.
//!
//! When the user toggles the overview (Ctrl+Alt+E), compd scales every open window down into a
//! near-square grid of live thumbnails over the dimmed work area; the pointer (hover) or the arrow
//! keys pick one, and activating it (Enter / a click) focuses+raises that window — restoring it if
//! minimized — and exits. This island owns the *mode + input edits*; the renderer (`renderer.rs`)
//! owns the pixels. The grid layout/hit-test/nav are the host-tested `wm_geom` geometry, so the only
//! thing that lives here is the compd-specific window-list ↔ grid-index mapping and the focus edit.

use crate::islands::{CompState, MAX_WINDOWS, PANEL_H};
use crate::messages::MouseSpatialMsg;

// Grid metrics in framebuffer pixels, shared with the renderer so a thumbnail's clickable cell and
// its drawn cell are the same region. `MARGIN` rings the whole grid, `GAP` separates cells, `PAD`
// insets the thumbnail inside its cell, `LABEL_H` reserves the caption strip at the cell's bottom.
pub const MARGIN: i32 = 28;
pub const GAP: i32 = 20;
pub const PAD: i32 = 10;
pub const LABEL_H: i32 = 18;

/// The overview canvas: the work area (framebuffer minus the bottom taskbar), so the panel stays
/// visible above the dim — the same region the snap zones tile into.
pub fn area(state: &CompState) -> wm_geom::Rect {
    wm_geom::Rect {
        x: 0,
        y: 0,
        w: state.fb_w as i32,
        h: (state.fb_h as i32 - PANEL_H as i32).max(1),
    }
}

/// The window slots shown in the overview, in a stable order (ascending slot index), so the grid
/// index `i` maps to the same window slot for both the renderer and the input hit-test. Every mapped
/// z1 app window is included — *including minimized ones*: surfacing them spatially (and restoring on
/// pick) is the whole point of an Exposé. The z0 desktop and empty slots are excluded.
pub fn slots(state: &CompState) -> ([usize; MAX_WINDOWS], usize) {
    let mut out = [0usize; MAX_WINDOWS];
    let mut n = 0usize;
    for (i, w) in state.windows.iter().enumerate() {
        if let Some(ref win) = w {
            if win.z_layer == 1 && win.mapped {
                out[n] = i;
                n += 1;
            }
        }
    }
    (out, n)
}

/// How many windows the overview grid holds (for `wm_geom::overview_nav`/`overview_hit`).
#[inline]
pub fn count(state: &CompState) -> u32 {
    slots(state).1 as u32
}

/// Toggle the overview on/off. Opening resets the selection to the first thumbnail and cancels any
/// in-progress window drag/snap-preview (the grid suspends direct window interaction). Closing just
/// clears the flag — the underlying windows are untouched.
pub fn toggle(state: &mut CompState) {
    if state.overview {
        state.overview = false;
        libmorpheus::debug!("overview toggle -> OFF");
        return;
    }
    state.overview = true;
    state.overview_sel = 0;
    state.capture = None;
    state.snap_preview = None;
    libmorpheus::debug!("overview toggle -> ON (n={})", count(state));
}

/// Move the keyboard selection within the grid (arrow keys), clamped to the populated cells.
pub fn nav(state: &mut CompState, dir: wm_geom::Dir) {
    let n = count(state);
    state.overview_sel = wm_geom::overview_nav(state.overview_sel, n, dir);
}

/// Activate the currently-selected thumbnail (Enter): focus+raise its window, restore it if it was
/// minimized, and exit the overview. A no-op selection (empty grid) just exits.
pub fn activate_selected(state: &mut CompState) {
    activate_index(state, state.overview_sel);
}

/// Focus+raise the window at grid index `gi` (restoring a minimized one) and exit the overview. An
/// out-of-range index simply exits. Raising is the same edit the rest of the WM uses — the renderer
/// composites `state.focused` last, on top.
pub fn activate_index(state: &mut CompState, gi: u32) {
    let (list, n) = slots(state);
    state.overview = false;
    if (gi as usize) >= n {
        return;
    }
    let idx = list[gi as usize];
    let pid = state.windows[idx].as_ref().map(|w| w.pid).unwrap_or(0);
    if let Some(w) = state.windows[idx].as_mut() {
        w.minimized = false; // surfacing a minimized window from the overview restores it.
        w.mouse_local_valid = false; // re-seed the app cursor on the next sample.
    }
    state.focused = Some(idx);
    libmorpheus::debug!("overview pick win {}", pid);
}

/// Handle a pointer sample while the overview is open: hover sets the selection (so the keyboard and
/// pointer share one highlighted thumbnail), and a left press on a thumbnail activates it. A press
/// that misses every cell (the outer margin, an inter-cell gap, or the still-visible panel) is inert
/// — the user can move onto a thumbnail or press Esc / Ctrl+Alt+E to dismiss. The normal window
/// hit-test/drag path is suspended while the overview is up (the caller returns early into here).
pub fn on_mouse(state: &mut CompState, msg: &MouseSpatialMsg) {
    let n = count(state);
    let a = area(state);
    if let Some(i) = wm_geom::overview_hit(n, a, MARGIN, GAP, msg.mx, msg.my) {
        state.overview_sel = i;
        if msg.left_pressed {
            activate_index(state, i);
        }
    }
}
