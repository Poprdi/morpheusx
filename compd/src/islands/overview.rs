//! Overview (Exposé) mode — mode flag, window↔grid mapping, and input edits. Renderer owns pixels.

use crate::islands::{CompState, MAX_WINDOWS, PANEL_H};
use crate::messages::MouseSpatialMsg;

// Grid metrics shared with the renderer (drawn cell == clickable cell).
pub const MARGIN: i32 = 28;
pub const GAP: i32 = 20;
pub const PAD: i32 = 10;
pub const LABEL_H: i32 = 18;

/// Work area (fb minus panel); the dim and thumbnails render inside this.
pub fn area(state: &CompState) -> wm_geom::Rect {
    wm_geom::Rect {
        x: 0,
        y: 0,
        w: state.fb_w as i32,
        h: (state.fb_h as i32 - PANEL_H as i32).max(1),
    }
}

/// Stable-order (ascending slot index) list of all mapped z1 windows, including minimized ones.
/// Grid index `i` must map to the same slot in both the renderer and the hit-test.
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

#[inline]
pub fn count(state: &CompState) -> u32 {
    slots(state).1 as u32
}

/// Toggle the overview. Opening resets selection to 0 and cancels any live drag/snap-preview.
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

/// Move the keyboard selection within the grid, clamped to populated cells.
pub fn nav(state: &mut CompState, dir: wm_geom::Dir) {
    let n = count(state);
    state.overview_sel = wm_geom::overview_nav(state.overview_sel, n, dir);
}

/// Focus+raise the selected thumbnail and exit; empty grid just exits.
pub fn activate_selected(state: &mut CompState) {
    activate_index(state, state.overview_sel);
}

/// Focus+raise window at grid index `gi`, restoring if minimized; out-of-range just exits.
pub fn activate_index(state: &mut CompState, gi: u32) {
    let (list, n) = slots(state);
    state.overview = false;
    if (gi as usize) >= n {
        return;
    }
    let idx = list[gi as usize];
    let pid = state.windows[idx].as_ref().map(|w| w.pid).unwrap_or(0);
    if let Some(w) = state.windows[idx].as_mut() {
        w.minimized = false;
        w.mouse_local_valid = false; // re-seed the app cursor on the next sample.
    }
    state.focused = Some(idx);
    libmorpheus::debug!("overview pick win {}", pid);
}

/// Handle pointer input while the overview is open: hover sets the selection, left press activates.
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
