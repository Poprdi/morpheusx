use crate::islands::{CompState, WIN_STATE_CAP, MAX_WINDOWS};
use crate::messages::InputMsg;

pub fn process_msgs(state: &mut CompState) {
    while let Some(msg) = state.ch_input_to_focus.recv() {
        match msg {
            InputMsg::FocusCycleRequest { reverse } => {
                cycle_focus(state, reverse);
            },
            InputMsg::WindowClosed { idx, .. } => {
                if state.focused == Some(idx as usize) {
                    state.focused = topmost_visible(state);
                }
            },
        }
    }
}

/// A window slot is focusable when it is a visible (non-minimized) z1 app window — the z0 desktop is
/// never focusable, and a minimized window is hidden so focus skips over it.
#[inline]
pub(crate) fn focusable(state: &CompState, i: usize) -> bool {
    matches!(state.windows[i], Some(ref w) if w.z_layer == 1 && !w.minimized)
}

/// The topmost (highest-slot) visible z1 app window, or `None` when none qualify — the fallback when
/// the focused window closes or is minimized.
fn topmost_visible(state: &CompState) -> Option<usize> {
    (0..MAX_WINDOWS).rev().find(|&i| focusable(state, i))
}

/// Move focus to the next (or previous, when `reverse`) visible z1 app window, wrapping around the
/// slot ring; the z0 desktop and minimized windows are never focusable. The ring-stepping order is
/// the host-tested `wm_geom::next_focus`. Focus also *raises* the window — the renderer composites
/// the focused window last (on top) — so Alt+Tab brings the next window to the front. A no-op when no
/// visible z1 window is focusable (focus stays put).
fn cycle_focus(state: &mut CompState, reverse: bool) {
    if let Some(next) = wm_geom::next_focus(state.focused, MAX_WINDOWS, reverse, |i| {
        focusable(state, i)
    }) {
        state.focused = Some(next);
    }
}

/// Service the desktop shell's taskbar-chip activation (`de.focus.req`): the shell leaves a
/// monotonic-token request when the user clicks a chip; compd reads it each frame and applies the
/// canonical taskbar policy (`wm_geom::chip_action`) from the target window's current state —
/// restore a minimized window, minimize the active one, or focus + raise any other. The token is
/// baselined at startup (see `main`), so a stale cross-boot value is ignored; only a strictly-new
/// token acts, and a pid that has since exited is a harmless no-op.
///
/// Focus/visibility is compd's domain — the shell only *asks*; the decision and the edit live here.
pub fn consume_focus_request(state: &mut CompState) {
    let (token, pid) = read_focus_request();
    if token == state.focus_req_token {
        return; // nothing new since the last frame (also covers the startup baseline).
    }
    state.focus_req_token = token;
    if pid == 0 {
        return;
    }

    // Find the z1 app window owning `pid`.
    let Some(idx) = state.windows.iter().position(|w| {
        matches!(w, Some(win) if win.z_layer == 1 && win.pid == pid)
    }) else {
        return;
    };

    let (is_min, is_focused) = {
        let w = state.windows[idx].as_ref().unwrap();
        (w.minimized, state.focused == Some(idx))
    };

    match wm_geom::chip_action(is_min, is_focused) {
        wm_geom::ChipAction::Restore => {
            if let Some(w) = state.windows[idx].as_mut() {
                w.minimized = false;
                w.mouse_local_valid = false; // re-seed the app cursor on the next sample.
            }
            state.focused = Some(idx);
            libmorpheus::debug!("restore win {}", pid);
        },
        wm_geom::ChipAction::Minimize => {
            if let Some(w) = state.windows[idx].as_mut() {
                w.minimized = true;
            }
            // The active window just hid itself → hand focus to the next visible window (or none).
            state.focused =
                wm_geom::next_focus(Some(idx), MAX_WINDOWS, false, |i| focusable(state, i));
            libmorpheus::debug!("minimize win {}", pid);
        },
        wm_geom::ChipAction::Focus => {
            state.focused = Some(idx);
            libmorpheus::debug!("focus win {}", pid);
        },
    }
}

/// Publish the current focus + minimized snapshot to the shell over the persist store
/// (`de.win.state`) so the taskbar chips reflect it (active stands out, hidden dims). Read-only for
/// the shell. Encodes `[focused_pid u32 LE][n_min u32 LE][min_pid u32 LE]…`; only writes (and incurs
/// the persist fsync) when the snapshot actually changed since the last frame.
pub fn publish_window_state(state: &mut CompState) {
    let mut buf = [0u8; WIN_STATE_CAP];

    let focused_pid = state
        .focused
        .and_then(|i| state.windows[i].as_ref())
        .map(|w| w.pid)
        .unwrap_or(0);
    buf[0..4].copy_from_slice(&focused_pid.to_le_bytes());

    let mut n_min = 0u32;
    let mut at = 8usize;
    for w in state.windows.iter().flatten() {
        if w.z_layer == 1 && w.minimized && at + 4 <= buf.len() {
            buf[at..at + 4].copy_from_slice(&w.pid.to_le_bytes());
            at += 4;
            n_min += 1;
        }
    }
    buf[4..8].copy_from_slice(&n_min.to_le_bytes());
    let len = at;

    // Only republish on change — the persist put fsyncs, so a per-frame write would be wasteful.
    if state.win_state_len == len && state.win_state_buf[..len] == buf[..len] {
        return;
    }
    state.win_state_buf = buf;
    state.win_state_len = len;
    let _ = libmorpheus::persist::put("de.win.state", &buf[..len]);
}

/// Read the shell's focus request (`de.focus.req`) as `(token, pid)`; a missing/short value reads as
/// `(0, 0)`. The blob is `[token u32 LE][pid u32 LE]` — the same contract `platform_morpheusx`
/// writes; the two processes share the byte layout, not the code.
pub fn read_focus_request() -> (u32, u32) {
    let mut buf = [0u8; 8];
    match libmorpheus::persist::get("de.focus.req", &mut buf) {
        Ok(n) if n >= 8 => {
            let token = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let pid = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            (token, pid)
        },
        _ => (0, 0),
    }
}
