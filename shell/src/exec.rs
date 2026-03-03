extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libmorpheus::fs;
use libmorpheus::process;

use crate::builtin;
use crate::parse::{Pipeline, SimpleCommand};
use crate::path;

pub fn run(pipeline: &Pipeline, cwd: &str) -> i32 {
    let n = pipeline.commands.len();
    if n == 1 {
        return run_single(&pipeline.commands[0], cwd);
    }
    run_pipeline(&pipeline.commands, cwd)
}

fn run_single(cmd: &SimpleCommand, cwd: &str) -> i32 {
    let has_redirect = cmd.stdout_file.is_some() || cmd.stdin_file.is_some();

    // If there are redirects, try running via builtin with fd-level I/O
    // (the serial dispatch uses fd 1, so redirects work naturally)
    if has_redirect {
        if let Some(first) = cmd.argv.first() {
            // Check if it's a builtin before doing path lookup
            if is_builtin(first) {
                let saved_in = save_fd(0);
                let saved_out = save_fd(1);

                if let Err(code) = setup_redirects(cmd, cwd) {
                    restore_fd(saved_in, 0);
                    restore_fd(saved_out, 1);
                    return code;
                }

                let result = builtin::dispatch(&cmd.argv, cwd).unwrap_or(1);

                restore_fd(saved_in, 0);
                restore_fd(saved_out, 1);

                if result == builtin::EXIT_SENTINEL {
                    return result;
                }

                if cmd.stdout_file.is_some() && result == 0 {
                    let _ = libmorpheus::fs::sync();
                }
                return result;
            }
        }
    }

    let binary = match path::which(&cmd.argv[0], cwd) {
        Some(p) => p,
        None => {
            libmorpheus::eprintln!("msh: command not found: {}", cmd.argv[0]);
            return 127;
        }
    };

    let saved_in = save_fd(0);
    let saved_out = save_fd(1);

    if let Err(code) = setup_redirects(cmd, cwd) {
        restore_fd(saved_in, 0);
        restore_fd(saved_out, 1);
        return code;
    }

    let result = spawn_and_wait(&binary, &cmd.argv);

    restore_fd(saved_in, 0);
    restore_fd(saved_out, 1);

    if has_redirect && result == 0 {
        let _ = libmorpheus::fs::sync();
    }

    result
}

fn run_pipeline(commands: &[SimpleCommand], cwd: &str) -> i32 {
    let n = commands.len();
    let saved_in = save_fd(0);
    let saved_out = save_fd(1);

    let mut pipes: Vec<(u32, u32)> = Vec::with_capacity(n - 1);
    for _ in 0..n - 1 {
        match process::pipe() {
            Ok(p) => pipes.push(p),
            Err(e) => {
                libmorpheus::eprintln!("msh: pipe: error 0x{:x}", e);
                restore_fd(saved_in, 0);
                restore_fd(saved_out, 1);
                return 1;
            }
        }
    }

    let has_redirects = commands.iter().any(|c| c.stdout_file.is_some());

    let mut children: Vec<u32> = Vec::with_capacity(n);

    for (i, cmd) in commands.iter().enumerate() {
        // Wire stdin from previous pipe
        if i > 0 {
            let _ = process::dup2(pipes[i - 1].0, 0);
        }

        // Wire stdout to next pipe
        if i < n - 1 {
            let _ = process::dup2(pipes[i].1, 1);
        } else {
            // Last command: restore original stdout
            if let Some(ref saved) = saved_out {
                let _ = process::dup2(*saved, 1);
            }
        }

        // Handle per-command redirects
        let _ = setup_redirects(cmd, cwd);

        // Try builtin first — serial dispatch writes to fd 1, so pipe redirects work
        if let Some(_code) = builtin::dispatch(&cmd.argv, cwd) {
            // Builtin ran, output went through fd 1 into the pipe
        } else {
            let binary = match path::which(&cmd.argv[0], cwd) {
                Some(p) => p,
                None => {
                    libmorpheus::eprintln!("msh: command not found: {}", cmd.argv[0]);
                    // Restore fds before continuing
                    if let Some(ref s) = saved_in {
                        let _ = process::dup2(*s, 0);
                    }
                    if let Some(ref s) = saved_out {
                        let _ = process::dup2(*s, 1);
                    }
                    continue;
                }
            };

            if let Some(pid) = spawn_child(&binary, &cmd.argv) { children.push(pid) }
        }

        // Restore shell's own fds for next iteration
        if let Some(ref s) = saved_in {
            let _ = process::dup2(*s, 0);
        }
        if let Some(ref s) = saved_out {
            let _ = process::dup2(*s, 1);
        }
    }

    // Close all pipe fds in the shell
    for (r, w) in &pipes {
        let _ = fs::close(*r as usize);
        let _ = fs::close(*w as usize);
    }

    // Wait for all children, return last exit code
    let mut last_status = 0i32;
    for pid in &children {
        match process::wait(*pid) {
            Ok(code) => last_status = code,
            Err(_) => last_status = 1,
        }
    }

    restore_fd(saved_in, 0);
    restore_fd(saved_out, 1);

    if has_redirects && last_status == 0 {
        let _ = libmorpheus::fs::sync();
    }

    last_status
}

fn spawn_and_wait(binary: &str, argv: &[String]) -> i32 {
    let my_pid = process::getpid();
    let args: Vec<&str> = argv.iter().skip(1).map(|s| s.as_str()).collect();

    let pid = match process::spawn_with_args(binary, &args) {
        Ok(pid) => pid,
        Err(e) => {
            libmorpheus::eprintln!("msh: spawn {}: error 0x{:x}", binary, e);
            return 126;
        }
    };

    process::set_foreground(pid);

    let status = process::wait(pid).unwrap_or(1);

    process::set_foreground(my_pid);
    status
}

fn spawn_child(binary: &str, argv: &[String]) -> Option<u32> {
    let args: Vec<&str> = argv.iter().skip(1).map(|s| s.as_str()).collect();
    match process::spawn_with_args(binary, &args) {
        Ok(pid) => Some(pid),
        Err(e) => {
            libmorpheus::eprintln!("msh: spawn {}: error 0x{:x}", binary, e);
            None
        }
    }
}

/// Spawn a child for compositor mode.
///
/// Creates a pipe so the compositor can route keyboard input to the child's
/// stdin, then spawns the process.  Does NOT call `set_foreground` or `wait` —
/// the compositor loop handles input routing and child lifecycle.
///
/// fd 0 is normally **not** in the fd_table for the shell (reads fall through
/// to the kernel ring buffer).  We temporarily place the pipe read-end there
/// via `dup2(rfd, 0)` so the child inherits it, then `close(0)` after spawn
/// to clear the fd_table entry — restoring the shell's ring-buffer stdin.
///
/// Returns `(pid, pipe_write_fd)` on success.
pub fn spawn_composited(binary: &str, args: &[&str]) -> Option<(u32, u32)> {
    // Create pipe: child reads from rfd, compositor writes to wfd.
    let (rfd, wfd) = match process::pipe() {
        Ok(p) => p,
        Err(e) => {
            libmorpheus::eprintln!("msh: pipe: error 0x{:x}", e);
            return None;
        }
    };

    // Place the pipe read-end at fd 0 for child inheritance.
    let _ = process::dup2(rfd, 0);

    let pid = match process::spawn_with_args(binary, args) {
        Ok(pid) => pid,
        Err(e) => {
            // Clean up: remove pipe from fd 0, close both pipe ends.
            let _ = fs::close(0);
            let _ = fs::close(rfd as usize);
            let _ = fs::close(wfd as usize);
            libmorpheus::eprintln!("msh: spawn {}: error 0x{:x}", binary, e);
            return None;
        }
    };

    // Remove pipe read-end from our fd 0 — shell reverts to ring-buffer stdin.
    let _ = fs::close(0);

    // Close the original rfd in our process — the child has it via fd 0.
    let _ = fs::close(rfd as usize);

    Some((pid, wfd))
}

fn setup_redirects(cmd: &SimpleCommand, cwd: &str) -> Result<(), i32> {
    if let Some(ref file) = cmd.stdin_file {
        let p = path::resolve(cwd, file);
        match fs::open(&p, fs::O_READ) {
            Ok(fd) => {
                let _ = process::dup2(fd as u32, 0);
                let _ = fs::close(fd);
            }
            Err(_) => {
                libmorpheus::eprintln!("msh: {}: cannot open for reading", p);
                return Err(1);
            }
        }
    }

    if let Some(ref redir) = cmd.stdout_file {
        let p = path::resolve(cwd, &redir.path);
        let flags = if redir.append {
            fs::O_WRITE | fs::O_CREATE | fs::O_APPEND
        } else {
            fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC
        };
        match fs::open(&p, flags) {
            Ok(fd) => {
                let _ = process::dup2(fd as u32, 1);
                let _ = fs::close(fd);
            }
            Err(_) => {
                libmorpheus::eprintln!("msh: {}: cannot open for writing", p);
                return Err(1);
            }
        }
    }

    Ok(())
}

fn save_fd(fd: u32) -> Option<u32> {
    fs::dup(fd as usize).ok().map(|d| d as u32)
}

fn restore_fd(saved: Option<u32>, target: u32) {
    if let Some(fd) = saved {
        let _ = process::dup2(fd, target);
        let _ = fs::close(fd as usize);
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "exit"
            | "quit"
            | "cd"
            | "pwd"
            | "echo"
            | "clear"
            | "true"
            | "false"
            | "help"
            | "ls"
            | "cat"
            | "mkdir"
            | "rm"
            | "rmdir"
            | "mv"
            | "cp"
            | "touch"
            | "stat"
            | "write"
            | "sync"
            | "ps"
            | "kill"
            | "sysinfo"
            | "sleep"
    )
}
