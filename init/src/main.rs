#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{entry, io, process};

mod islands;

entry!(main);

fn main() -> i32 {
    io::println("init: starting MorpheusX Desktop Environment");

    let mut state = islands::supervisor::SupervisorState::new();

    // spawn compd
    match process::spawn("/bin/compd") {
        Ok(pid) => {
            io::println("init: spawned compd");
            state.compd_pid = Some(pid);
        }
        Err(_) => {
            io::println("init: FATAL — failed to spawn compd");
        }
    }

    // spawn shelld
    match process::spawn("/bin/shelld") {
        Ok(pid) => {
            io::println("init: spawned shelld");
            state.shelld_pid = Some(pid);
        }
        Err(_) => {
            io::println("init: FATAL — failed to spawn shelld");
        }
    }

    // install SIGCHLD handler (signal 17)
    let _ = process::sigaction(
        process::signal::SIGCHLD,
        sigchld_handler as *const () as u64,
    );

    // supervisor loop
    loop {
        islands::supervisor::tick(&mut state);
        process::yield_cpu();
    }
}

extern "C" fn sigchld_handler() {
    // handled in supervisor::tick via SYS_TRY_WAIT. the handler just unblocks us.
    // no allocation inside signal handlers. invariant B6.
    process::sigreturn();
}
