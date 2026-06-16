//! Window context menu — compd's consumer of `wm_geom::menu_*`.
//!
//! Right-clicking a window's title bar opens a small popup of that window's operations
//! (maximize/restore, minimize, the snap halves, close). This island owns the *menu's lifetime +
//! the input edits*; the renderer (`renderer.rs`) owns the pixels. The popup's box/rows/hit-test/nav
//! are the host-tested `wm_geom::menu_*` geometry, so the only thing here is the compd-specific
//! window↔menu binding and dispatching each row onto the edit compd already performs — a snap reuses
//! `input::apply_snap`, Minimize reuses `input::minimize_window`, Close reuses `input::close_window`.
//! The menu adds a discoverable surface for those operations, never new behaviour.

use crate::islands::{CompState, PANEL_H};
use crate::messages::MouseSpatialMsg;

/// Pixel metrics for the popup, shared with the renderer so a row's clickable band and its drawn
/// band are the same region. `char_w` is the 8px font cell (labels are sized to it); `min_w` floors
/// the box so a short menu still reads as a panel.
pub const METRICS: wm_geom::MenuMetrics = wm_geom::MenuMetrics {
    row_h: 22,
    sep_h: 7,
    pad_x: 14,
    char_w: 8,
    min_w: 150,
};

/// An open window context menu: the window it operates on, its on-screen box, its rows (snapshotted
/// at open time so the Maximize/Restore label stays stable while the menu is up), and the highlighted
/// row. `Copy` so it can be read out of `state.menu` without holding a borrow during dispatch.
#[derive(Clone, Copy)]
pub struct WindowMenu {
    /// The window slot the menu acts on (validated live before every dispatch — a window can be
    /// reaped while its menu is open).
    pub target: usize,
    pub ox: i32,
    pub oy: i32,
    pub w: i32,
    pub h: i32,
    pub rows: [wm_geom::MenuRow; wm_geom::WINDOW_MENU_ROWS],
    /// Index into `rows` of the highlighted item (always an item row, never a separator).
    pub sel: usize,
}

/// Open the context menu for window slot `target`, anchored at the pointer `(ax, ay)` and clamped on
/// screen. The first row reads `Restore` when the window is maximized (`saved_rect.is_some()`) and
/// `Maximize` otherwise — the same toggle the `[□]/[❐]` title button drives. A no-op for a slot that
/// is not a live z1 app window. Opening cancels any in-progress drag/snap-preview (the menu suspends
/// direct window interaction, like the overview does).
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

/// Dismiss the menu with no change (Esc, or a click/right-click outside the box).
#[inline]
pub fn dismiss(state: &mut CompState) {
    state.menu = None;
}

/// Move the highlighted item up/down with the arrow keys, skipping separators and wrapping like a
/// real menu. A no-op when no menu is open.
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

/// Run the action of row `row_i` and close the menu. A separator/out-of-range index closes the menu
/// with no action. The action maps onto the edit compd already has — a snap zone via
/// `input::apply_snap` (Maximize/Restore is the saved_rect toggle), `input::minimize_window`, or
/// `input::close_window` — so a menu pick lands a window in the exact state its keyboard/title-button
/// equivalent would. The target window is re-validated (it may have been reaped) before any edit.
pub fn dispatch_row(state: &mut CompState, row_i: usize) {
    let Some(m) = state.menu else {
        return;
    };
    state.menu = None; // a pick always dismisses, whatever the row.
    let (action, label) = match m.rows.get(row_i) {
        Some(wm_geom::MenuRow::Item { action, label }) => (*action, *label),
        _ => return, // separator or out of range.
    };
    // The menu's window may have closed while it was open — re-check before editing.
    let pid = match state.windows[m.target] {
        Some(ref w) if w.z_layer == 1 => w.pid,
        _ => return,
    };
    // One line per pick, mirroring the drag/snap/minimize readouts — the menu is the new surface,
    // so make its dispatch observable on the serial console too.
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

/// If a menu is open for a window that has since been unmapped/reaped, dismiss it. Called each frame
/// so a stale popup never lingers over a dead window.
pub fn prune(state: &mut CompState) {
    if let Some(m) = state.menu {
        if !matches!(state.windows[m.target], Some(ref w) if w.z_layer == 1 && w.mapped) {
            state.menu = None;
        }
    }
}

/// Handle a pointer sample while the menu is open: hover highlights the row under the pointer, a left
/// press on a row runs it, and a left/right press outside the box dismisses. The normal window
/// hit-test/drag path is suspended while the menu is up (the caller returns early into here).
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
