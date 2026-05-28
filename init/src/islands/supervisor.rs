use libmorpheus::{compositor as compsys, io, process};

// pid table is 64 slots. we stop restarting before we eat them all.
const MAX_RESTARTS: u32 = 5;

pub struct SupervisorState {
    pub compd_pid: Option<u32>,
    pub shelld_pid: Option<u32>,
    pub compd_restarts: u32,
    pub shelld_restarts: u32,
}

impl SupervisorState {
    pub fn new() -> Self {
        Self {
            compd_pid: None,
            shelld_pid: None,
            compd_restarts: 0,
            shelld_restarts: 0,
        }
    }
}

// Input is owned by the compositor. compd drains the kernel keyboard event
// ring via SYS_KEYBOARD_READ and handles global hotkeys (e.g. Ctrl+Alt+X →
// spawn /bin/msh). init does NOT read input: once compd registers as the
// compositor, init is a "composited client" whose per-process input buffer is
// never fed, so a stdin read here would block forever and stall supervision.
// init is now a pure daemon supervisor.
pub fn tick(state: &mut SupervisorState) {
    // SIGCHLD means a child died. We check who and restart them.

    // check compd
    if let Some(pid) = state.compd_pid {
        if let Ok(Some(_code)) = process::try_wait(pid) {
            state.compd_pid = None;
            if state.compd_restarts < MAX_RESTARTS {
                state.compd_restarts += 1;
                // reclaim compositor slot before re-spawning. invariant D1.
                let _ = compsys::compositor_set();
                match process::spawn("/bin/compd") {
                    Ok(new_pid) => state.compd_pid = Some(new_pid),
                    Err(_) => io::println("init: failed to respawn compd"),
                }
            } else {
                io::println("init: compd exceeded MAX_RESTARTS, giving up");
            }
        }
    }

    // check shelld
    if let Some(pid) = state.shelld_pid {
        if let Ok(Some(_code)) = process::try_wait(pid) {
            state.shelld_pid = None;
            if state.shelld_restarts < MAX_RESTARTS {
                state.shelld_restarts += 1;
                match process::spawn("/bin/shelld") {
                    Ok(new_pid) => state.shelld_pid = Some(new_pid),
                    Err(_) => io::println("init: failed to respawn shelld"),
                }
            } else {
                io::println("init: shelld exceeded MAX_RESTARTS, giving up");
            }
        }
    }
}
