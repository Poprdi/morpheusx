mod fs_cmds;
mod help;
mod proc_cmds;

extern crate alloc;

use alloc::format;
use alloc::string::String;

use crate::console::Console;
use crate::fb::Framebuffer;
use crate::path;

pub const EXIT_SENTINEL: i32 = i32::MIN;

static mut PREV_DIR: [u8; 256] = [0u8; 256];
static mut PREV_DIR_LEN: usize = 0;

fn save_prev_dir(cwd: &str) {
    let bytes = cwd.as_bytes();
    let len = bytes.len().min(255);
    unsafe {
        PREV_DIR[..len].copy_from_slice(&bytes[..len]);
        PREV_DIR_LEN = len;
    }
}

fn get_prev_dir() -> Option<String> {
    unsafe {
        if PREV_DIR_LEN == 0 {
            None
        } else {
            core::str::from_utf8(&PREV_DIR[..PREV_DIR_LEN])
                .ok()
                .map(String::from)
        }
    }
}

pub fn dispatch(argv: &[String], cwd: &str) -> Option<i32> {
    if argv.is_empty() {
        return None;
    }

    let cmd = argv[0].as_str();
    let args = &argv[1..];

    if has_help_flag(args) {
        if let Some(text) = help::usage(cmd) {
            libmorpheus::io::print(text);
            return Some(0);
        }
    }

    match cmd {
        "exit" | "quit" => Some(exit_cmd(args)),
        "cd" => Some(cd_cmd(args, cwd)),
        "pwd" => Some(pwd_cmd()),
        "echo" => Some(echo_cmd(args)),
        "clear" => Some(clear_cmd()),
        "true" => Some(0),
        "false" => Some(1),
        "help" => Some(help_cmd(args)),

        "ls" => Some(fs_cmds::ls(args, cwd)),
        "cat" => Some(fs_cmds::cat(args, cwd)),
        "mkdir" => Some(fs_cmds::mkdir(args, cwd)),
        "rm" => Some(fs_cmds::rm(args, cwd)),
        "mv" => Some(fs_cmds::mv(args, cwd)),
        "cp" => Some(fs_cmds::cp(args, cwd)),
        "touch" => Some(fs_cmds::touch(args, cwd)),
        "stat" => Some(fs_cmds::stat(args, cwd)),
        "write" => Some(fs_cmds::write(args, cwd)),
        "sync" => Some(fs_cmds::sync_cmd()),

        "ps" => Some(proc_cmds::ps()),
        "kill" => Some(proc_cmds::kill(args)),
        "sysinfo" => Some(proc_cmds::sysinfo()),
        "sleep" => Some(proc_cmds::sleep(args)),

        _ => None,
    }
}

pub fn dispatch_fb(
    argv: &[String],
    cwd: &str,
    fb: &Framebuffer,
    con: &mut Console,
) -> Option<i32> {
    if argv.is_empty() {
        return None;
    }

    let cmd = argv[0].as_str();
    let args = &argv[1..];

    if has_help_flag(args) {
        if let Some(text) = help::usage(cmd) {
            con.write_str(fb, text);
            return Some(0);
        }
    }

    match cmd {
        "exit" | "quit" => Some(exit_cmd(args)),
        "cd" => Some(cd_cmd_fb(args, cwd, fb, con)),
        "pwd" => Some(pwd_cmd_fb(fb, con)),
        "echo" => Some(echo_cmd_fb(args, fb, con)),
        "clear" => {
            con.clear(fb);
            Some(0)
        }
        "true" => Some(0),
        "false" => Some(1),
        "help" => Some(help_cmd_fb(args, fb, con)),

        "ls" => Some(fs_cmds::ls_fb(args, cwd, fb, con)),
        "cat" => Some(fs_cmds::cat_fb(args, cwd, fb, con)),
        "mkdir" => Some(fs_cmds::mkdir_fb(args, cwd, fb, con)),
        "rm" => Some(fs_cmds::rm_fb(args, cwd, fb, con)),
        "mv" => Some(fs_cmds::mv_fb(args, cwd, fb, con)),
        "cp" => Some(fs_cmds::cp_fb(args, cwd, fb, con)),
        "touch" => Some(fs_cmds::touch_fb(args, cwd, fb, con)),
        "stat" => Some(fs_cmds::stat_fb(args, cwd, fb, con)),
        "write" => Some(fs_cmds::write_fb(args, cwd, fb, con)),
        "sync" => Some(fs_cmds::sync_cmd_fb(fb, con)),

        "ps" => Some(proc_cmds::ps_fb(fb, con)),
        "kill" => Some(proc_cmds::kill_fb(args, fb, con)),
        "sysinfo" => Some(proc_cmds::sysinfo_fb(fb, con)),
        "sleep" => Some(proc_cmds::sleep_fb(args, fb, con)),

        _ => None,
    }
}

fn has_help_flag(args: &[String]) -> bool {
    args.iter().any(|a| a == "--help" || a == "-h")
}

fn exit_cmd(args: &[String]) -> i32 {
    if let Some(s) = args.first() {
        if let Some(code) = parse_i32(s) {
            unsafe {
                core::ptr::write_volatile(&raw mut EXIT_CODE_OVERRIDE, code);
            }
        }
    }
    EXIT_SENTINEL
}

static mut EXIT_CODE_OVERRIDE: i32 = 0;

pub fn exit_code() -> i32 {
    unsafe { core::ptr::read_volatile(&raw const EXIT_CODE_OVERRIDE) }
}

fn cd_cmd(args: &[String], cwd: &str) -> i32 {
    let target = match args.first().map(|s| s.as_str()) {
        None => String::from("/"),
        Some("-") => match get_prev_dir() {
            Some(p) => {
                libmorpheus::println!("{}", p);
                p
            }
            None => {
                libmorpheus::eprintln!("cd: OLDPWD not set");
                return 1;
            }
        },
        Some(p) => path::resolve(cwd, p),
    };
    // Root always exists — skip metadata check
    if target != "/" {
        match libmorpheus::fs::metadata(&target) {
            Ok(m) => {
                if !m.is_dir() {
                    libmorpheus::eprintln!("cd: {}: Not a directory", target);
                    return 1;
                }
            }
            Err(_) => {
                libmorpheus::eprintln!("cd: {}: No such directory", target);
                return 1;
            }
        }
    }
    save_prev_dir(cwd);
    match libmorpheus::env::set_current_dir(&target) {
        Ok(()) => 0,
        Err(e) => {
            libmorpheus::eprintln!("cd: {}: {}", target, e);
            1
        }
    }
}

fn cd_cmd_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let target = match args.first().map(|s| s.as_str()) {
        None => String::from("/"),
        Some("-") => match get_prev_dir() {
            Some(p) => {
                con.write_str(fb, &p);
                con.write_str(fb, "\n");
                p
            }
            None => {
                con.write_colored(fb, "cd: OLDPWD not set\n", (170, 0, 0));
                return 1;
            }
        },
        Some(p) => path::resolve(cwd, p),
    };
    // Root always exists — skip metadata check
    if target != "/" {
        match libmorpheus::fs::metadata(&target) {
            Ok(m) => {
                if !m.is_dir() {
                    con.write_colored(fb, &format!("cd: {}: Not a directory\n", target), (170, 0, 0));
                    return 1;
                }
            }
            Err(_) => {
                con.write_colored(fb, &format!("cd: {}: No such directory\n", target), (170, 0, 0));
                return 1;
            }
        }
    }
    save_prev_dir(cwd);
    match libmorpheus::env::set_current_dir(&target) {
        Ok(()) => 0,
        Err(e) => {
            con.write_colored(fb, &format!("cd: {}: {}\n", target, e), (170, 0, 0));
            1
        }
    }
}

fn pwd_cmd() -> i32 {
    match libmorpheus::env::current_dir() {
        Ok(cwd) => {
            libmorpheus::println!("{}", cwd);
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("pwd: {}", e);
            1
        }
    }
}

fn pwd_cmd_fb(fb: &Framebuffer, con: &mut Console) -> i32 {
    match libmorpheus::env::current_dir() {
        Ok(cwd) => {
            con.write_str(fb, &cwd);
            con.write_str(fb, "\n");
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("pwd: {}\n", e), (170, 0, 0));
            1
        }
    }
}

fn echo_cmd(args: &[String]) -> i32 {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            libmorpheus::io::print(" ");
        }
        libmorpheus::io::print(arg);
    }
    libmorpheus::io::print("\n");
    0
}

fn echo_cmd_fb(args: &[String], fb: &Framebuffer, con: &mut Console) -> i32 {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            con.write_str(fb, " ");
        }
        con.write_str(fb, arg);
    }
    con.write_str(fb, "\n");
    0
}

fn clear_cmd() -> i32 {
    libmorpheus::io::print("\x1b[2J\x1b[H");
    0
}

fn help_cmd(args: &[String]) -> i32 {
    if let Some(cmd) = args.first() {
        match help::usage(cmd.as_str()) {
            Some(text) => {
                libmorpheus::io::print(text);
                return 0;
            }
            None => {
                libmorpheus::eprintln!("help: unknown command: {}", cmd);
                return 1;
            }
        }
    }

    libmorpheus::io::print(HELP_TEXT);
    0
}

fn help_cmd_fb(args: &[String], fb: &Framebuffer, con: &mut Console) -> i32 {
    if let Some(cmd) = args.first() {
        match help::usage(cmd.as_str()) {
            Some(text) => {
                con.write_str(fb, text);
                return 0;
            }
            None => {
                con.write_colored(
                    fb,
                    &format!("help: unknown command: {}\n", cmd),
                    (170, 0, 0),
                );
                return 1;
            }
        }
    }

    con.write_str(fb, HELP_TEXT);
    0
}

const HELP_TEXT: &str = concat!(
    "msh - MorpheusX Shell\n",
    "\n",
    "Builtins:\n",
    "  cd [path]       Change directory\n",
    "  pwd             Print working directory\n",
    "  echo [args]     Print arguments\n",
    "  clear           Clear screen\n",
    "  exit [code]     Exit shell\n",
    "  help [cmd]      Show help (for a command)\n",
    "\n",
    "Filesystem:\n",
    "  ls [-l] [path]  List directory\n",
    "  cat <file>      Display file\n",
    "  mkdir <path>    Create directory\n",
    "  rm <path>       Remove file\n",
    "  mv <src> <dst>  Rename/move\n",
    "  cp <src> <dst>  Copy file\n",
    "  touch <path>    Create empty file\n",
    "  stat <path>     File metadata\n",
    "  write <p> <t>   Write text to file\n",
    "  sync            Flush journal\n",
    "\n",
    "Process:\n",
    "  ps              List processes\n",
    "  kill <pid> [s]  Send signal (default: 15)\n",
    "  sysinfo         System information\n",
    "  sleep <ms>      Sleep milliseconds\n",
    "\n",
    "Operators:\n",
    "  cmd1 | cmd2     Pipeline\n",
    "  cmd < file      Redirect stdin\n",
    "  cmd > file      Redirect stdout\n",
    "  cmd >> file     Append stdout\n",
    "\n",
    "Use 'help <cmd>' or '<cmd> --help' for details.\n",
);

fn parse_i32(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (neg, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };
    if start >= bytes.len() {
        return None;
    }
    let mut val: i32 = 0;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i32)?;
    }
    Some(if neg { -val } else { val })
}

pub fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

pub fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut val: u64 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(val)
}
