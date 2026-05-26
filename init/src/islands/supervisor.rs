use libmorpheus::{compositor as compsys, io, process};

// pid table is 64 slots. we stop restarting before we eat them all.
const MAX_RESTARTS: u32 = 5;

// PS/2 Set 1 byte protocol — what the bootloader now pushes directly into
// stdin (no character decoding upstream).
const EXTENDED_PREFIX: u8 = 0xE0;
const BREAK_FLAG: u8 = 0x80;

// Set 1 scancodes for what we currently care about.
const SC_LCTRL: u8 = 0x1D;
const SC_LSHIFT: u8 = 0x2A;
const SC_RSHIFT: u8 = 0x36;
const SC_LALT: u8 = 0x38;
const SC_X: u8 = 0x2D;

pub struct SupervisorState {
    pub compd_pid: Option<u32>,
    pub shelld_pid: Option<u32>,
    pub compd_restarts: u32,
    pub shelld_restarts: u32,
    /// PID of the user-launched msh, or None when no shell is running.
    /// While Some, init stops consuming stdin so the shell gets keystrokes.
    pub user_shell_pid: Option<u32>,

    // Keyboard scancode state machine. Tracks the 0xE0 extended prefix and
    // the three modifiers init currently observes (Ctrl/Shift/Alt). Both
    // left and right modifier scancodes feed the same boolean — userland
    // doesn't currently need to distinguish sides for hotkey purposes.
    pub kbd_extended: bool,
    pub kbd_ctrl: bool,
    pub kbd_shift: bool,
    pub kbd_alt: bool,
}

impl SupervisorState {
    pub fn new() -> Self {
        Self {
            compd_pid: None,
            shelld_pid: None,
            compd_restarts: 0,
            shelld_restarts: 0,
            user_shell_pid: None,
            kbd_extended: false,
            kbd_ctrl: false,
            kbd_shift: false,
            kbd_alt: false,
        }
    }
}

pub fn tick(state: &mut SupervisorState) {
    // SIGCHLD means a child died. very literal. we check who and restart them. circle of life.

    // check compd
    if let Some(pid) = state.compd_pid {
        match process::try_wait(pid) {
            Ok(Some(_code)) => {
                state.compd_pid = None;
                if state.compd_restarts < MAX_RESTARTS {
                    state.compd_restarts += 1;
                    // reclaim compositor slot before re-spawning. invariant D1.
                    let _ = compsys::compositor_set();
                    match process::spawn("/bin/compd") {
                        Ok(new_pid) => state.compd_pid = Some(new_pid),
                        Err(_) => {
                            libmorpheus::io::println("init: failed to respawn compd");
                        }
                    }
                } else {
                    libmorpheus::io::println("init: compd exceeded MAX_RESTARTS, giving up");
                }
            }
            _ => {} // still running or error
        }
    }

    // check shelld
    if let Some(pid) = state.shelld_pid {
        match process::try_wait(pid) {
            Ok(Some(_code)) => {
                state.shelld_pid = None;
                if state.shelld_restarts < MAX_RESTARTS {
                    state.shelld_restarts += 1;
                    match process::spawn("/bin/shelld") {
                        Ok(new_pid) => state.shelld_pid = Some(new_pid),
                        Err(_) => {
                            libmorpheus::io::println("init: failed to respawn shelld");
                        }
                    }
                } else {
                    libmorpheus::io::println("init: shelld exceeded MAX_RESTARTS, giving up");
                }
            }
            _ => {}
        }
    }

    // Hotkey listener. Init reads raw PS/2 Set 1 scancodes from stdin —
    // the bootloader no longer decodes characters; it just pushes the raw
    // byte stream (with 0xE0 extended prefix and 0x80 break-bit). We track
    // modifier state here and recognize Ctrl+X as the spawn-shell hotkey.
    //
    // Future work (TODO): apply a keyboard layout (US/DE/etc.) to the
    // non-modifier keys, route the resulting characters through a pipe to
    // the spawned shell so msh sees actual text instead of raw scancodes.
    // Layout will then be runtime-configurable from settings.
    if let Some(pid) = state.user_shell_pid {
        match process::try_wait(pid) {
            Ok(Some(_code)) => {
                state.user_shell_pid = None;
                // Foreground = 0 so Ctrl+C ends up as a plain stdin byte
                // again (no SIGINT routed) — init will read it on the next
                // tick. Modifier state is *not* reset here because the user
                // might still be physically holding Shift/Ctrl/Alt across
                // the shell-exit transition; the next scancode keeps the
                // state machine consistent.
                process::set_foreground(0);
                io::println("init: user shell exited; Ctrl+X to spawn another");
            }
            _ => {}
        }
    } else {
        // Non-blocking read: SYS_READ on stdin returns 0 if the queue is
        // empty. Process up to a small batch of scancodes per tick so the
        // state machine catches up if the user mashed keys.
        let mut buf = [0u8; 16];
        let n = io::read_stdin(&mut buf);
        for &byte in &buf[..n] {
            if handle_scancode(state, byte) {
                // Hotkey fired — spawn shell and stop processing this tick.
                spawn_user_shell(state);
                break;
            }
        }
    }
}

/// Feed one PS/2 Set 1 byte into the modifier state machine. Returns true
/// when the byte completes the Ctrl+X spawn-shell hotkey sequence.
fn handle_scancode(state: &mut SupervisorState, byte: u8) -> bool {
    if byte == EXTENDED_PREFIX {
        state.kbd_extended = true;
        return false;
    }

    let is_break = (byte & BREAK_FLAG) != 0;
    let make = byte & !BREAK_FLAG;
    let was_extended = state.kbd_extended;
    state.kbd_extended = false;

    // Right-side Ctrl/Alt arrive via 0xE0 prefix; left-side directly.
    if was_extended {
        match make {
            SC_LCTRL => {
                state.kbd_ctrl = !is_break;
            }
            SC_LALT => {
                state.kbd_alt = !is_break;
            }
            _ => {}
        }
        return false;
    }

    match make {
        SC_LCTRL => {
            state.kbd_ctrl = !is_break;
        }
        SC_LSHIFT | SC_RSHIFT => {
            state.kbd_shift = !is_break;
        }
        SC_LALT => {
            state.kbd_alt = !is_break;
        }
        SC_X => {
            // Spawn-shell hotkey: fire only on the press edge with Ctrl held.
            if !is_break && state.kbd_ctrl {
                return true;
            }
        }
        _ => {}
    }
    false
}

fn spawn_user_shell(state: &mut SupervisorState) {
    io::println("init: Ctrl+X — spawning /bin/msh");
    match process::spawn("/bin/msh") {
        Ok(new_pid) => {
            state.user_shell_pid = Some(new_pid);
            // Foreground = shell so Ctrl+C from inside the shell would
            // (once we wire it back up) deliver SIGINT to msh, not init.
            process::set_foreground(new_pid);
        }
        Err(_) => io::println("init: failed to spawn /bin/msh"),
    }
}
