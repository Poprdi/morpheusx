use libmorpheus::{process, compositor as compsys};

// pid table is 64 slots. we stop restarting before we eat them all.
const MAX_RESTARTS: u32 = 5;

pub struct SupervisorState {
    pub compd_pid:       Option<u32>,
    pub shelld_pid:      Option<u32>,
    pub compd_restarts:  u32,
    pub shelld_restarts: u32,
}

impl SupervisorState {
    pub fn new() -> Self {
        Self {
            compd_pid:       None,
            shelld_pid:      None,
            compd_restarts:  0,
            shelld_restarts: 0,
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
}
