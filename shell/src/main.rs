#![no_std]
#![no_main]

extern crate alloc;

mod builtin;
mod console;
mod exec;
mod fb;
mod font;
mod line;
mod parse;
mod path;
mod prompt;

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use libmorpheus::entry;
use libmorpheus::env;
use libmorpheus::process;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

entry!(main);

fn main() -> i32 {
    install_signals();

    let framebuffer = match fb::Framebuffer::init() {
        Some(fb) => fb,
        None => {
            libmorpheus::io::print("msh: failed to map framebuffer\n");
            return 1;
        }
    };

    let mut con = console::Console::new(&framebuffer);
    con.clear(&framebuffer);

    con.write_colored(&framebuffer, "msh 1.0", (85, 255, 85));
    con.write_str(&framebuffer, " - MorpheusX Shell\n");
    con.write_str(&framebuffer, "Type 'help' for commands.\n\n");

    let mut last_status: i32 = 0;

    loop {
        let cwd = env::current_dir().unwrap_or_else(|_| String::from("/"));
        con.render_prompt(&framebuffer, &cwd, last_status);

        INTERRUPTED.store(false, Ordering::Release);

        let prompt_col = con.cursor_col();

        let input = match line::read_line_fb(&framebuffer, &mut con, prompt_col, &|| {
            INTERRUPTED.load(Ordering::Acquire)
        }) {
            Some(l) => l,
            None => {
                con.write_str(&framebuffer, "^C\n");
                last_status = 130;
                continue;
            }
        };

        con.newline(&framebuffer);

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Ctrl+L: clear and restart
        if trimmed == "\x0c" {
            con.clear(&framebuffer);
            continue;
        }

        let pipeline = match parse::parse(trimmed) {
            Some(p) => p,
            None => continue,
        };

        if pipeline.commands.len() == 1 {
            let cmd = &pipeline.commands[0];
            let has_redirects = cmd.stdin_file.is_some() || cmd.stdout_file.is_some();

            // If there are redirects, always go through exec (handles fd-level I/O)
            if !has_redirects {
                if let Some(code) = builtin::dispatch_fb(&cmd.argv, &cwd, &framebuffer, &mut con) {
                    if code == builtin::EXIT_SENTINEL {
                        return builtin::exit_code();
                    }
                    last_status = code;
                    continue;
                }
            }
        }

        let status = exec::run(&pipeline, &cwd);
        if status == 127 {
            let cmd_name = pipeline
                .commands
                .first()
                .and_then(|c| c.argv.first())
                .map(|s| s.as_str())
                .unwrap_or("(unknown)");
            con.write_colored(&framebuffer, cmd_name, (255, 85, 85));
            con.write_str(&framebuffer, ": not a known command\n");
            last_status = 0;
        } else {
            last_status = status;
        }
    }
}

fn install_signals() {
    extern "C" fn sigint_handler(_sig: u64) {
        INTERRUPTED.store(true, Ordering::Release);
        process::sigreturn();
    }

    let _ = process::sigaction(process::signal::SIGINT, sigint_handler as *const () as u64);
}
