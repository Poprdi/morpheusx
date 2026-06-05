#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{entry, error, info, process};

mod islands;

entry!(main);

fn main() -> i32 {
    info!("starting MorpheusX Desktop Environment");

    let mut state = islands::supervisor::SupervisorState::new();

    match process::spawn("/bin/compd") {
        Ok(pid) => {
            info!("spawned compd pid={}", pid);
            state.compd_pid = Some(pid);
        },
        Err(e) => {
            error!("failed to spawn compd: {:#x}", e);
        },
    }

    match process::spawn("/bin/shelld") {
        Ok(pid) => {
            info!("spawned shelld pid={}", pid);
            state.shelld_pid = Some(pid);
        },
        Err(e) => {
            error!("failed to spawn shelld: {:#x}", e);
        },
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
