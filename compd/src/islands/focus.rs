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

/// Focusable: z1, mapped, not minimized. z0 desktop and minimized slots are excluded.
#[inline]
pub(crate) fn focusable(state: &CompState, i: usize) -> bool {
    matches!(state.windows[i], Some(ref w) if w.z_layer == 1 && !w.minimized)
}

/// Highest-index focusable slot; fallback when the focused window closes or is minimized.
fn topmost_visible(state: &CompState) -> Option<usize> {
    (0..MAX_WINDOWS).rev().find(|&i| focusable(state, i))
}

/// Step focus to the next (or previous) z1 window via `wm_geom::next_focus`; raises on focus.
fn cycle_focus(state: &mut CompState, reverse: bool) {
    if let Some(next) = wm_geom::next_focus(state.focused, MAX_WINDOWS, reverse, |i| {
        focusable(state, i)
    }) {
        state.focused = Some(next);
    }
}

/// Service `de.focus.req` each frame: applies `wm_geom::chip_action` for the requesting pid.
/// Token baselined at startup so stale cross-boot requests are ignored.
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

/// Write `de.win.state` (`[focused_pid u32 LE][n_min u32 LE][min_pid…]`) only when the snapshot
/// changed; avoids fsyncing every frame on an idle desktop.
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

/// Read `de.focus.req` as `(token, pid)`. Blob: `[token u32 LE][pid u32 LE]`.
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
