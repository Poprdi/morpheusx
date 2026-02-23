use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

// ═══════════════════════════════════════════════════════════════════════
// FsOp — filesystem operation descriptors
// ═══════════════════════════════════════════════════════════════════════

/// Filesystem operation requested by a shell command.
///
/// The desktop event loop (which has VFS access) interprets these
/// and pushes the result text back into the shell output.
///
/// HelixFS has NO file permissions — no rwx, no uid/gid.
/// All paths are pre-resolved to absolute form by the command parser
/// using the shell's current working directory.
pub enum FsOp {
    /// List directory contents. `long` shows type, size, version count.
    Ls { path: String, long: bool },
    /// Change working directory. Executor must verify target is a directory.
    Cd { path: String },
    /// Create a directory.
    Mkdir { path: String },
    /// Create an empty file (no-op if already exists).
    Touch { path: String },
    /// Read and display file contents (UTF-8 text; rejects binary).
    Cat { path: String },
    /// Remove a file or empty directory.
    Rm { path: String },
    /// Rename / move.
    Mv { src: String, dst: String },
    /// Write text content to a file (creates or overwrites).
    Write { path: String, content: String },
    /// Display HelixFS metadata (key, size, LSN, versions, flags).
    Stat { path: String },
    /// Flush the log-structured journal to disk.
    Sync,
}

pub enum CommandResult {
    Output(String),
    Clear,
    OpenApp(String),
    CloseWindow(u32),
    ListWindows,
    SpawnProcess(String),
    FsCommand(FsOp),
    Exit,
    Unknown(String),
}

pub fn execute(input: &str, _window_ids: &[u32], cwd: &str) -> CommandResult {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return CommandResult::Output(String::new());
    }

    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match cmd {
        "help" => CommandResult::Output(help_text()),
        "clear" => CommandResult::Clear,
        "exit" | "quit" => CommandResult::Exit,
        "open" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: open <app-name>"))
            } else {
                CommandResult::OpenApp(String::from(arg))
            }
        }
        "exec" | "run" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: exec <binary-name>"))
            } else {
                CommandResult::SpawnProcess(String::from(arg))
            }
        }
        "close" => {
            if let Some(id) = parse_u32(arg) {
                CommandResult::CloseWindow(id)
            } else {
                CommandResult::Output(String::from("Usage: close <window-id>"))
            }
        }
        "list" | "windows" => CommandResult::ListWindows,

        // ── Filesystem commands ─────────────────────────────────────
        "pwd" => CommandResult::Output(String::from(cwd)),

        "cd" => {
            if arg.is_empty() {
                CommandResult::FsCommand(FsOp::Cd {
                    path: String::from("/"),
                })
            } else {
                CommandResult::FsCommand(FsOp::Cd {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "ls" => {
            let (long, path_arg) = parse_ls_args(arg);
            let path = if path_arg.is_empty() {
                String::from(cwd)
            } else {
                resolve_path(cwd, path_arg)
            };
            CommandResult::FsCommand(FsOp::Ls { path, long })
        }

        "mkdir" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: mkdir <path>"))
            } else {
                CommandResult::FsCommand(FsOp::Mkdir {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "touch" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: touch <path>"))
            } else {
                CommandResult::FsCommand(FsOp::Touch {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "cat" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: cat <path>"))
            } else {
                CommandResult::FsCommand(FsOp::Cat {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "rm" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: rm <path>"))
            } else {
                CommandResult::FsCommand(FsOp::Rm {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "mv" => {
            let mut mv_parts = arg.splitn(2, ' ');
            let src = mv_parts.next().unwrap_or("").trim();
            let dst = mv_parts.next().unwrap_or("").trim();
            if src.is_empty() || dst.is_empty() {
                CommandResult::Output(String::from("Usage: mv <source> <destination>"))
            } else {
                CommandResult::FsCommand(FsOp::Mv {
                    src: resolve_path(cwd, src),
                    dst: resolve_path(cwd, dst),
                })
            }
        }

        "write" => {
            let mut w_parts = arg.splitn(2, ' ');
            let path = w_parts.next().unwrap_or("").trim();
            let content = w_parts.next().unwrap_or("");
            if path.is_empty() {
                CommandResult::Output(String::from("Usage: write <path> <content>"))
            } else {
                CommandResult::FsCommand(FsOp::Write {
                    path: resolve_path(cwd, path),
                    content: String::from(content),
                })
            }
        }

        "stat" => {
            if arg.is_empty() {
                CommandResult::Output(String::from("Usage: stat <path>"))
            } else {
                CommandResult::FsCommand(FsOp::Stat {
                    path: resolve_path(cwd, arg),
                })
            }
        }

        "sync" => CommandResult::FsCommand(FsOp::Sync),

        _ => CommandResult::Unknown(String::from(cmd)),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Path resolution
// ═══════════════════════════════════════════════════════════════════════

/// Resolve a possibly-relative path against the current working directory.
///
/// Handles `.` (current), `..` (parent), and normalizes slashes.
pub fn resolve_path(cwd: &str, input: &str) -> String {
    let raw = if input.starts_with('/') {
        String::from(input)
    } else {
        let mut full = String::from(cwd);
        if !full.ends_with('/') {
            full.push('/');
        }
        full.push_str(input);
        full
    };
    normalize_path(&raw)
}

/// Normalize an absolute path: collapse `.`, `..`, and extra slashes.
fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    if components.is_empty() {
        return String::from("/");
    }
    let mut result = String::with_capacity(path.len());
    for c in &components {
        result.push('/');
        result.push_str(c);
    }
    result
}

/// Parse `ls` arguments. Returns `(long_format, path_argument)`.
///
/// Supports: `ls`, `ls -l`, `ls <path>`, `ls -l <path>`.
fn parse_ls_args(arg: &str) -> (bool, &str) {
    if arg.is_empty() {
        return (false, "");
    }
    let mut tokens = arg.split_whitespace();
    let first = tokens.next().unwrap_or("");
    if first == "-l" {
        (true, tokens.next().unwrap_or(""))
    } else {
        (false, first)
    }
}

pub fn format_window_list(ids: &[u32]) -> String {
    if ids.is_empty() {
        return String::from("No open windows.");
    }
    let mut out = String::from("Open windows:\n");
    for &id in ids {
        out.push_str(&format!("  [{}]\n", id));
    }
    out
}

fn help_text() -> String {
    String::from(
        "Commands:\n\
         \x20 help            Show this help\n\
         \x20 clear           Clear output\n\
         \x20 open <app>      Open an application\n\
         \x20 exec <name>     Spawn a user process\n\
         \x20 close <id>      Close window by ID\n\
         \x20 list            List open windows\n\
         \n\
         Filesystem:\n\
         \x20 pwd             Print working directory\n\
         \x20 cd [path]       Change directory\n\
         \x20 ls [-l] [path]  List directory contents\n\
         \x20 mkdir <path>    Create directory\n\
         \x20 touch <path>    Create empty file\n\
         \x20 cat <path>      Display file contents\n\
         \x20 write <p> <txt> Write text to file\n\
         \x20 rm <path>       Remove file or empty dir\n\
         \x20 mv <src> <dst>  Rename / move\n\
         \x20 stat <path>     HelixFS metadata\n\
         \x20 sync            Flush log to disk\n\
         \n\
         \x20 exit            Halt system",
    )
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for &b in s.as_bytes() {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}
