//! Window context menu — lifetime, input dispatch, and `wm_geom::menu_*` binding.
//! The renderer owns the pixels; this island owns the compd-specific window↔menu binding.

use crate::islands::{CompState, PANEL_H};
use crate::messages::MouseSpatialMsg;

/// Popup metrics shared with the renderer (drawn band == clickable band).
pub const METRICS: wm_geom::MenuMetrics = wm_geom::MenuMetrics {
    row_h: 22,
    sep_h: 7,
    pad_x: 14,
    char_w: 8,
    min_w: 150,
};

/// Open window context menu. `Copy` so it can be read out of `state.menu` without a live borrow.
/// `target` is re-validated before every dispatch — a window can be reaped while the menu is open.
#[derive(Clone, Copy)]
pub struct WindowMenu {
    pub target: usize,
    pub ox: i32,
    pub oy: i32,
    pub w: i32,
    pub h: i32,
    pub rows: [wm_geom::MenuRow; wm_geom::WINDOW_MENU_ROWS],
    pub sel: usize, // highlighted row index (always an item, never a separator).
}

/// Open the context menu for `target` anchored at `(ax, ay)`, clamped on screen.
/// No-op for a non-live z1 slot; cancels any in-progress drag/snap-preview.
pub fn open(state: &mut CompState, target: usize, ax: i32, ay: i32) {
    let (pid, maximized) = match state.windows[target].as_ref() {
        Some(w) if w.z_layer == 1 => (w.pid, w.saved_rect.is_some()),
        _ => return,
    };
    let rows = wm_geom::window_menu(maximized);
    let (w, h) = wm_geom::menu_size(&rows, METRICS);
    let (ox, oy) = wm_geom::menu_origin(
        ax,
        ay,
        w,
        h,
        state.fb_w as i32,
        state.fb_h as i32,
        PANEL_H as i32,
    );
    state.menu = Some(WindowMenu {
        target,
        ox,
        oy,
        w,
        h,
        rows,
        sel: 0, // row 0 is always the Maximize/Restore item.
    });
    state.capture = None;
    state.snap_preview = None;
    libmorpheus::debug!("menu open win {} @ {},{}", pid, ox, oy);
}

/// Dismiss the menu with no action.
#[inline]
pub fn dismiss(state: &mut CompState) {
    state.menu = None;
}

/// Move the highlight up/down, skipping separators and wrapping.
pub fn nav(state: &mut CompState, down: bool) {
    if let Some(ref mut m) = state.menu {
        m.sel = wm_geom::menu_nav(m.sel, &m.rows, down);
    }
}

/// Run the highlighted row (Enter).
pub fn dispatch_selected(state: &mut CompState) {
    if let Some(m) = state.menu {
        dispatch_row(state, m.sel);
    }
}

/// Run the action of `row_i` and close the menu; separator or out-of-range closes with no action.
/// The target window is re-validated before any edit (may have been reaped while open).
pub fn dispatch_row(state: &mut CompState, row_i: usize) {
    let Some(m) = state.menu else {
        return;
    };
    state.menu = None;
    let (action, label) = match m.rows.get(row_i) {
        Some(wm_geom::MenuRow::Item { action, label }) => (*action, *label),
        _ => return, // separator or out of range.
    };
    let pid = match state.windows[m.target] {
        Some(ref w) if w.z_layer == 1 => w.pid,
        _ => return,
    };
    libmorpheus::debug!("menu pick win {}: {}", pid, label);
    match action {
        wm_geom::MenuAction::Minimize => crate::islands::input::minimize_window(state, m.target),
        wm_geom::MenuAction::Close => crate::islands::input::close_window(state, m.target),
        snap => {
            if let Some(zone) = wm_geom::menu_action_zone(snap) {
                crate::islands::input::apply_snap(state, m.target, zone);
            }
        },
    }
}

/// Dismiss a stale menu if its target window was reaped; called each frame.
pub fn prune(state: &mut CompState) {
    if let Some(m) = state.menu {
        if !matches!(state.windows[m.target], Some(ref w) if w.z_layer == 1 && w.mapped) {
            state.menu = None;
        }
    }
}

/// Handle pointer input while the menu is open: hover highlights, left press runs, outside press dismisses.
pub fn on_mouse(state: &mut CompState, msg: &MouseSpatialMsg) {
    let Some(m) = state.menu else {
        return;
    };
    let hit = wm_geom::menu_hit(&m.rows, METRICS, m.ox, m.oy, m.w, msg.mx, msg.my);
    if let Some(i) = hit {
        if let Some(ref mut menu) = state.menu {
            menu.sel = i;
        }
    }
    if msg.left_pressed {
        match hit {
            Some(i) => dispatch_row(state, i),
            None => dismiss(state), // click outside the box dismisses.
        }
    } else if msg.right_pressed && hit.is_none() {
        dismiss(state); // a right-click elsewhere also cancels.
    }
}
