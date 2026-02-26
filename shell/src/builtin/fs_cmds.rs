extern crate alloc;

use alloc::string::String;

use crate::path;

pub fn ls(args: &[String], cwd: &str) -> i32 {
    let (long, target) = parse_ls_args(args, cwd);
    let entries = match libmorpheus::fs::read_dir(&target) {
        Ok(e) => e,
        Err(e) => {
            libmorpheus::eprintln!("ls: {}: {}", target, e);
            return 1;
        }
    };

    for entry in entries {
        if long {
            let kind = if entry.is_dir() { "DIR " } else { "FILE" };
            libmorpheus::println!(
                "{} {:>10}  v{:<3} {}",
                kind,
                entry.size(),
                entry.version_count(),
                entry.name()
            );
        } else {
            let suffix = if entry.is_dir() { "/" } else { "" };
            libmorpheus::print!("{}{}  ", entry.name(), suffix);
        }
    }

    if !long {
        libmorpheus::io::print("\n");
    }
    0
}

fn parse_ls_args<'a>(args: &'a [String], cwd: &str) -> (bool, String) {
    let mut long = false;
    let mut path_arg: Option<&'a str> = None;

    for a in args {
        if a == "-l" {
            long = true;
        } else if path_arg.is_none() {
            path_arg = Some(a.as_str());
        }
    }

    let target = match path_arg {
        Some(p) => path::resolve(cwd, p),
        None => String::from(cwd),
    };
    (long, target)
}

pub fn cat(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("cat: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::read_to_string(&p) {
        Ok(content) => {
            libmorpheus::io::print(&content);
            if !content.ends_with('\n') {
                libmorpheus::io::print("\n");
            }
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("cat: {}: {}", p, e);
            1
        }
    }
}

pub fn mkdir(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("mkdir: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::create_dir(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("mkdir: {}: {}", p, e);
            1
        }
    }
}

pub fn rm(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("rm: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::remove_file(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("rm: {}: {}", p, e);
            1
        }
    }
}

pub fn mv(args: &[String], cwd: &str) -> i32 {
    if args.len() < 2 {
        libmorpheus::eprintln!("mv: need <src> <dst>");
        return 1;
    }
    let src = path::resolve(cwd, &args[0]);
    let dst = path::resolve(cwd, &args[1]);
    match libmorpheus::fs::rename_path(&src, &dst) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("mv: {}", e);
            1
        }
    }
}

pub fn cp(args: &[String], cwd: &str) -> i32 {
    if args.len() < 2 {
        libmorpheus::eprintln!("cp: need <src> <dst>");
        return 1;
    }
    let src = path::resolve(cwd, &args[0]);
    let dst = path::resolve(cwd, &args[1]);
    match libmorpheus::fs::copy(&src, &dst) {
        Ok(_) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("cp: {}", e);
            1
        }
    }
}

pub fn touch(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("touch: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if libmorpheus::fs::metadata(&p).is_ok() {
        return 0;
    }
    match libmorpheus::fs::write_bytes(&p, b"") {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("touch: {}: {}", p, e);
            1
        }
    }
}

pub fn stat(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("stat: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::metadata(&p) {
        Ok(m) => {
            libmorpheus::println!("  Path: {}", p);
            libmorpheus::println!("  Type: {}", if m.is_dir() { "directory" } else { "file" });
            libmorpheus::println!("  Size: {} bytes", m.len());
            libmorpheus::println!("   Key: 0x{:016x}", m.key);
            libmorpheus::println!("   LSN: {} (first: {})", m.lsn, m.first_lsn);
            libmorpheus::println!("  Vers: {}", m.version_count);
            libmorpheus::println!(" Flags: 0x{:08x}", m.flags);
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("stat: {}: {}", p, e);
            1
        }
    }
}

pub fn write(args: &[String], cwd: &str) -> i32 {
    if args.len() < 2 {
        libmorpheus::eprintln!("write: need <path> <text>");
        return 1;
    }
    let p = path::resolve(cwd, &args[0]);
    let text = join_args(&args[1..]);
    match libmorpheus::fs::write_bytes(&p, text.as_bytes()) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("write: {}: {}", p, e);
            1
        }
    }
}

pub fn sync_cmd() -> i32 {
    match libmorpheus::fs::sync() {
        Ok(()) => {
            libmorpheus::println!("synced");
            0
        }
        Err(e) => {
            libmorpheus::eprintln!("sync: {:?}", e);
            1
        }
    }
}

fn join_args(args: &[String]) -> String {
    let total: usize = args.iter().map(|a| a.len()).sum::<usize>() + args.len();
    let mut out = String::with_capacity(total);
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(a);
    }
    out
}
