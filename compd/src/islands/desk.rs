use crate::islands::{CompState, MAX_WINDOWS};

// Servicing the desktop (root) context menu's window commands. The shell owns the z0 wallpaper but
// *not* window state, so a pick of one of the menu's window-management rows arrives here as a
// versioned request over the `de.desk.cmd` persist key — the same shape as the taskbar focus request
// (`focus::consume_focus_request`): a monotonic token + the command's wire id. compd baselines the
// token at startup (so a stale cross-boot value never acts) and services only a strictly-newer one,
// mapping each command onto an operation it ALREADY owns — the overview it opens for Ctrl+Alt+E, the
// minimize the chip/title-button take — so the menu is a discoverable surface, not new behaviour.

/// The desktop-command wire ids. These MUST match `phosphor_de::DeskCommand::to_wire` (compd can't
/// import that enum; the integers are the cross-process contract, pinned host-side by
/// `desk_command_wire_values_are_stable`). `0` is reserved for "no command" (the cleared baseline).
const CMD_SHOW_ALL_WINDOWS: u32 = 1;
const CMD_MINIMIZE_ALL: u32 = 2;

/// Read `de.desk.cmd` as `(token, cmd)`. Blob: `[token u32 LE][cmd u32 LE]`.
pub fn read_desk_command() -> (u32, u32) {
    let mut buf = [0u8; 8];
    match libmorpheus::persist::get("de.desk.cmd", &mut buf) {
        Ok(n) if n >= 8 => {
            let token = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let cmd = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            (token, cmd)
        },
        _ => (0, 0),
    }
}

/// Service `de.desk.cmd` each frame; acts only on a strictly-new token; ignores unknown ids.
pub fn consume_desk_command(state: &mut CompState) {
    let (token, cmd) = read_desk_command();
    if token == state.desk_cmd_token {
        return; // nothing new since the last frame (also covers the startup baseline).
    }
    state.desk_cmd_token = token;

    match cmd {
        CMD_SHOW_ALL_WINDOWS => {
            // Guarded so a repeated pick re-opens rather than toggling a just-opened grid back off.
            if !state.overview {
                crate::islands::overview::toggle(state);
            }
            libmorpheus::debug!("desk cmd: show all windows");
        },
        CMD_MINIMIZE_ALL => {
            for i in 0..MAX_WINDOWS {
                if crate::islands::focus::focusable(state, i) {
                    crate::islands::input::minimize_window(state, i);
                }
            }
            libmorpheus::debug!("desk cmd: minimize all windows");
        },
        _ => {}, // 0 (cleared) or an unrecognised id: ignore.
    }
}
