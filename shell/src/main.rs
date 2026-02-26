#![no_std]
#![no_main]

extern crate alloc;

mod builtin;
mod exec;
mod line;
mod parse;
mod path;
mod prompt;

use core::sync::atomic::{AtomicBool, Ordering};

use libmorpheus::entry;
use libmorpheus::env;
use libmorpheus::io;
use libmorpheus::process;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

entry!(main);

fn main() -> i32 {
    install_signals();
    print_banner();

    let mut editor = line::LineEditor::new();
    let mut last_status: i32 = 0;

    loop {
        prompt::render(last_status);
        INTERRUPTED.store(false, Ordering::Release);

        let line = match editor.read_line(&|| INTERRUPTED.load(Ordering::Acquire)) {
            Some(l) => l,
            None => {
                // Ctrl+C: print ^C and restart
                io::print("^C\n");
                last_status = 130;
                continue;
            }
        };

        // Ctrl+L sentinel: redraw prompt
        if line == "\x0c" {
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let cwd = env::current_dir().unwrap_or_else(|_| alloc::string::String::from("/"));

        let pipeline = match parse::parse(trimmed) {
            Some(p) => p,
            None => continue,
        };

        // Try builtins first (only for single non-piped commands)
        if pipeline.commands.len() == 1 {
            if let Some(code) = builtin::dispatch(&pipeline.commands[0].argv, &cwd) {
                if code == builtin::EXIT_SENTINEL {
                    return builtin::exit_code();
                }
                last_status = code;
                continue;
            }
        }

        last_status = exec::run(&pipeline, &cwd);
    }
}

fn install_signals() {
    extern "C" fn sigint_handler(_sig: u64) {
        INTERRUPTED.store(true, Ordering::Release);
        process::sigreturn();
    }

    let _ = process::sigaction(
        process::signal::SIGINT,
        sigint_handler as *const () as u64,
    );
}

fn print_banner() {
    io::print("msh 1.0 — MorpheusX Shell\nType 'help' for commands.\n\n");
}
