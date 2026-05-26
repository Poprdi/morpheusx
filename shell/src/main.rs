#![no_std]
#![no_main]

extern crate alloc;

mod builtin;
mod compositor;
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

    let is_compositor = libmorpheus::compositor::compositor_set().is_ok();
    if !is_compositor {
        libmorpheus::io::print("msh: running in client mode\n");
    }

    let mut con = console::Console::new(&framebuffer);
    con.clear(&framebuffer);

    con.write_colored(&framebuffer, "msh 0.0.12", (85, 255, 85));
    con.write_str(&framebuffer, " - MorpheusX Shell\n");
    con.write_str(&framebuffer, "Type 'help' for commands.\n\n");

    let mut last_status: i32 = 0;
    let mut comp = compositor::Compositor::new(&framebuffer);

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

            // Redirects route through exec for fd-level I/O; bypass builtins.
            if !has_redirects {
                if let Some(code) = builtin::dispatch_fb(&cmd.argv, &cwd, &framebuffer, &mut con) {
                    if code == builtin::EXIT_SENTINEL {
                        return builtin::exit_code();
                    }
                    last_status = code;
                    continue;
                }
            }

            let binary = match path::which(&cmd.argv[0], &cwd) {
                Some(p) => p,
                None => {
                    con.write_colored(&framebuffer, &cmd.argv[0], (255, 85, 85));
                    con.write_str(&framebuffer, ": not a known command\n");
                    last_status = 0;
                    continue;
                }
            };

            if !is_compositor {
                last_status = exec::run(&pipeline, &cwd);
                continue;
            }

            let args: alloc::vec::Vec<&str> = cmd.argv.iter().skip(1).map(|s| s.as_str()).collect();

            match exec::spawn_composited(&binary, &args) {
                Some(pid) => {
                    comp.add_child(pid, &cmd.argv[0]);

                    // Returns when all children exit.
                    last_status = compositor::compositor_loop(&framebuffer, &mut comp);

                    // Repaint console only if compositor touched the framebuffer.
                    if comp.did_compose {
                        con.clear(&framebuffer);
                        comp.did_compose = false;
                    }
                }
                None => {
                    con.write_colored(&framebuffer, &cmd.argv[0], (255, 85, 85));
                    con.write_str(&framebuffer, ": failed to spawn\n");
                    last_status = 126;
                }
            }

            continue;
        }

        // Pipeline path: blocking exec, no compositor.
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
    let _ = process::sigaction(process::signal::SIGINT, 1);
}
