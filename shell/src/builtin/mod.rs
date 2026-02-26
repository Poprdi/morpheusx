mod fs_cmds;
mod help;
mod proc_cmds;

extern crate alloc;

use alloc::string::String;

use crate::path;

pub const EXIT_SENTINEL: i32 = i32::MIN;

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
    let target = match args.first() {
        Some(p) => path::resolve(cwd, p),
        None => String::from("/"),
    };
    match libmorpheus::env::set_current_dir(&target) {
        Ok(()) => 0,
        Err(e) => {
            libmorpheus::eprintln!("cd: {}: {}", target, e);
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

    libmorpheus::io::print(concat!(
        "msh — MorpheusX Shell\n",
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
    ));
    0
}

fn parse_i32(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (neg, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
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
