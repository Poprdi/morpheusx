use crate::islands::{CompState, MAX_WINDOWS};
use crate::messages::InputMsg;

pub fn process_msgs(state: &mut CompState) {
    while let Some(msg) = state.ch_input_to_focus.recv() {
        match msg {
            InputMsg::FocusCycleRequest => {
                cycle_focus(state);
            },
            InputMsg::WindowClosed { idx, .. } => {
                if state.focused == Some(idx as usize) {
                    // Refocus next z1 window (z0 desktop is not focusable).
                    state.focused = state
                        .windows
                        .iter()
                        .enumerate()
                        .find(|(_, w)| w.as_ref().map(|w| w.z_layer == 1).unwrap_or(false))
                        .map(|(i, _)| i);
                }
            },
        }
    }
}

fn cycle_focus(state: &mut CompState) {
    // Round-robin through z1 windows; skip z0 desktop.
    let start = state.focused.map(|f| f + 1).unwrap_or(0);
    for offset in 0..MAX_WINDOWS {
        let idx = (start + offset) % MAX_WINDOWS;
        if let Some(ref win) = state.windows[idx] {
            if win.z_layer == 1 {
                state.focused = Some(idx);
                return;
            }
        }
    }
}
