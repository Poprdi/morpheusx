use libmorpheus::{compositor as compsys, io, process};

// pid table is 64 slots. we stop restarting before we eat them all.
const MAX_RESTARTS: u32 = 5;

/// Hotkey byte that spawns an interactive shell on /bin/msh.
const HOTKEY_SPAWN_SHELL: u8 = 0x18; // Ctrl+X

pub struct SupervisorState {
    pub compd_pid: Option<u32>,
    pub shelld_pid: Option<u32>,
    pub compd_restarts: u32,
    pub shelld_restarts: u32,
    /// PID of the user-launched msh, or None when no shell is running.
    /// While Some, init stops consuming stdin so the shell gets keystrokes.
    pub user_shell_pid: Option<u32>,
}

impl SupervisorState {
    pub fn new() -> Self {
        Self {
            compd_pid: None,
            shelld_pid: None,
            compd_restarts: 0,
            shelld_restarts: 0,
            user_shell_pid: None,
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

    // Hotkey listener. While no user shell is running, init reads stdin and
    // watches for Ctrl+X. When pressed, spawn /bin/msh and hand foreground
    // (Ctrl+C's stdin signal target) over to the shell. While the shell is
    // alive init stops consuming stdin so all keystrokes flow to it. When
    // the shell exits we reclaim quiescent state and resume listening.
    if let Some(pid) = state.user_shell_pid {
        match process::try_wait(pid) {
            Ok(Some(_code)) => {
                state.user_shell_pid = None;
                // Foreground = 0: Ctrl+C reverts to being a plain byte in
                // the kernel stdin queue, which init will eat on the next
                // tick (and ignore unless it happens to be Ctrl+X again).
                process::set_foreground(0);
                io::println("init: user shell exited; Ctrl+X to spawn another");
            }
            _ => {}
        }
    } else {
        // Non-blocking read: SYS_READ on stdin returns 0 if the queue is
        // empty (per the read_line pattern in libmorpheus::io). Other
        // non-Ctrl+X bytes are consumed but unused — there's no point
        // forwarding them anywhere with no shell to receive them.
        let mut buf = [0u8; 1];
        if io::read_stdin(&mut buf) > 0 && buf[0] == HOTKEY_SPAWN_SHELL {
            io::println("init: Ctrl+X — spawning /bin/msh");
            match process::spawn("/bin/msh") {
                Ok(new_pid) => {
                    state.user_shell_pid = Some(new_pid);
                    // Foreground = shell so Ctrl+C from inside the shell
                    // delivers SIGINT to msh, not to init.
                    process::set_foreground(new_pid);
                }
                Err(_) => io::println("init: failed to spawn /bin/msh"),
            }
        }
    }
}
