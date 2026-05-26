#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{entry, io, process};

mod islands;

entry!(main);

fn main() -> i32 {
    io::println("init: starting MorpheusX Desktop Environment");

    let mut state = islands::supervisor::SupervisorState::new();

    match process::spawn("/bin/compd") {
        Ok(pid) => {
            io::println("init: spawned compd");
            state.compd_pid = Some(pid);
        }
        Err(_) => {
            io::println("init: FATAL — failed to spawn compd");
        }
    }

    match process::spawn("/bin/shelld") {
        Ok(pid) => {
            io::println("init: spawned shelld");
            state.shelld_pid = Some(pid);
        }
        Err(_) => {
            io::println("init: FATAL — failed to spawn shelld");
        }
    }

    let _ = process::sigaction(
        process::signal::SIGCHLD,
        sigchld_handler as *const () as u64,
    );

    loop {
        islands::supervisor::tick(&mut state);
        process::yield_cpu();
    }
}

extern "C" fn sigchld_handler() {
    // Real work happens in supervisor::tick via SYS_TRY_WAIT; this just unblocks us.
    // Invariant B6: no allocation inside signal handlers.
    process::sigreturn();
}
