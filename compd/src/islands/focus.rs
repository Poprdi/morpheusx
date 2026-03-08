use crate::islands::{CompState, MAX_WINDOWS};
use crate::messages::InputMsg;

pub fn process_msgs(state: &mut CompState) {
    while let Some(msg) = state.ch_input_to_focus.recv() {
        match msg {
            InputMsg::FocusCycleRequest => {
                cycle_focus(state);
            }
            InputMsg::WindowClosed { idx, .. } => {
                if state.focused == Some(idx as usize) {
                    // refocus next z1 window. desktop (z0) is not focusable.
                    state.focused = state.windows.iter().enumerate()
                        .find(|(_, w)| w.as_ref().map(|w| w.z_layer == 1).unwrap_or(false))
                        .map(|(i, _)| i);
                }
            }
        }
    }
}

fn cycle_focus(state: &mut CompState) {
    // round-robin through active z_layer 1 windows. desktop (z0) is not focusable.
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
