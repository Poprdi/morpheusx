extern crate alloc;

use alloc::format;
use alloc::string::String;

use crate::console::Console;
use crate::fb::Framebuffer;
use crate::path;

pub fn ls(args: &[String], cwd: &str) -> i32 {
    let (long, target) = parse_ls_args(args, cwd);
    // If target is a file, show just that entry
    if let Ok(m) = libmorpheus::fs::metadata(&target) {
        if m.is_file() {
            let name = path::basename(&target);
            if long {
                libmorpheus::println!("FILE {:>10}  v{:<3} {}", m.len(), m.version_count, name);
            } else {
                libmorpheus::println!("{}", name);
            }
            return 0;
        }
    }
    let entries = match libmorpheus::fs::read_dir(&target) {
        Ok(e) => e,
        Err(e) => {
            libmorpheus::eprintln!("ls: {}: {}", target, e);
            return 1;
        }
    };

    let mut count = 0u32;
    for entry in entries {
        count += 1;
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

    if !long && count > 0 {
        libmorpheus::io::print("\n");
    }
    0
}

pub fn ls_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let (long, target) = parse_ls_args(args, cwd);
    // If target is a file, show just that entry
    if let Ok(m) = libmorpheus::fs::metadata(&target) {
        if m.is_file() {
            let name = path::basename(&target);
            if long {
                con.write_str(
                    fb,
                    &format!("FILE {:>10}  v{:<3} {}\n", m.len(), m.version_count, name),
                );
            } else {
                con.write_str(fb, name);
                con.write_str(fb, "\n");
            }
            return 0;
        }
    }
    let entries = match libmorpheus::fs::read_dir(&target) {
        Ok(e) => e,
        Err(e) => {
            con.write_colored(fb, &format!("ls: {}: {}\n", target, e), (170, 0, 0));
            return 1;
        }
    };

    let mut count = 0u32;
    for entry in entries {
        count += 1;
        if long {
            let kind = if entry.is_dir() { "DIR " } else { "FILE" };
            con.write_str(
                fb,
                &format!(
                    "{} {:>10}  v{:<3} {}\n",
                    kind,
                    entry.size(),
                    entry.version_count(),
                    entry.name()
                ),
            );
        } else {
            let suffix = if entry.is_dir() { "/" } else { "" };
            let name = entry.name();
            if entry.is_dir() {
                con.write_colored(fb, name, (85, 85, 255));
                con.write_colored(fb, suffix, (85, 85, 255));
            } else {
                con.write_str(fb, name);
            }
            con.write_str(fb, "  ");
        }
    }

    if !long && count > 0 {
        con.write_str(fb, "\n");
    }
    0
}

fn parse_ls_args<'a>(args: &'a [String], cwd: &str) -> (bool, String) {
    let mut long = false;
    let mut path_arg: Option<&'a str> = None;

    for a in args {
        if a.starts_with('-') && a.len() > 1 && a.as_bytes()[1] != b'-' {
            // Parse combined short flags: -l, -la, -al, etc.
            for ch in a[1..].chars() {
                if ch == 'l' {
                    long = true;
                }
                // -a is accepted but no-op (we always show all files)
            }
        } else if a == "--long" {
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
    if args.is_empty() {
        libmorpheus::eprintln!("cat: missing operand");
        return 1;
    }
    let mut ret = 0;
    for arg in args {
        let p = path::resolve(cwd, arg);
        match libmorpheus::fs::metadata(&p) {
            Ok(m) if m.is_dir() => {
                libmorpheus::eprintln!("cat: {}: Is a directory", p);
                ret = 1;
                continue;
            }
            Err(e) => {
                libmorpheus::eprintln!("cat: {}: {}", p, e);
                ret = 1;
                continue;
            }
            _ => {}
        }
        match libmorpheus::fs::read_to_string(&p) {
            Ok(content) => {
                libmorpheus::io::print(&content);
                if !content.ends_with('\n') {
                    libmorpheus::io::print("\n");
                }
            }
            Err(e) => {
                libmorpheus::eprintln!("cat: {}: {}", p, e);
                ret = 1;
            }
        }
    }
    ret
}

pub fn cat_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    if args.is_empty() {
        con.write_colored(fb, "cat: missing operand\n", (170, 0, 0));
        return 1;
    }
    let mut ret = 0;
    for arg in args {
        let p = path::resolve(cwd, arg);
        match libmorpheus::fs::metadata(&p) {
            Ok(m) if m.is_dir() => {
                con.write_colored(fb, &format!("cat: {}: Is a directory\n", p), (170, 0, 0));
                ret = 1;
                continue;
            }
            Err(e) => {
                con.write_colored(fb, &format!("cat: {}: {}\n", p, e), (170, 0, 0));
                ret = 1;
                continue;
            }
            _ => {}
        }
        match libmorpheus::fs::read_to_string(&p) {
            Ok(content) => {
                con.write_str(fb, &content);
                if !content.ends_with('\n') {
                    con.write_str(fb, "\n");
                }
            }
            Err(e) => {
                con.write_colored(fb, &format!("cat: {}: {}\n", p, e), (170, 0, 0));
                ret = 1;
            }
        }
    }
    ret
}

pub fn mkdir(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("mkdir: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            libmorpheus::eprintln!("mkdir: {}: Directory already exists", p);
        } else {
            libmorpheus::eprintln!("mkdir: {}: File exists", p);
        }
        return 1;
    }
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

pub fn mkdir_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(arg) = args.first() else {
        con.write_colored(fb, "mkdir: missing operand\n", (170, 0, 0));
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            con.write_colored(fb, &format!("mkdir: {}: Directory already exists\n", p), (170, 0, 0));
        } else {
            con.write_colored(fb, &format!("mkdir: {}: File exists\n", p), (170, 0, 0));
        }
        return 1;
    }
    match libmorpheus::fs::create_dir(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("mkdir: {}: {}\n", p, e), (170, 0, 0));
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
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            libmorpheus::eprintln!("rm: {}: Is a directory", p);
            return 1;
        }
    }
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

pub fn rm_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(arg) = args.first() else {
        con.write_colored(fb, "rm: missing operand\n", (170, 0, 0));
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            con.write_colored(fb, &format!("rm: {}: Is a directory\n", p), (170, 0, 0));
            return 1;
        }
    }
    match libmorpheus::fs::remove_file(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("rm: {}: {}\n", p, e), (170, 0, 0));
            1
        }
    }
}

pub fn rmdir(args: &[String], cwd: &str) -> i32 {
    let Some(arg) = args.first() else {
        libmorpheus::eprintln!("rmdir: missing operand");
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::metadata(&p) {
        Ok(m) if !m.is_dir() => {
            libmorpheus::eprintln!("rmdir: {}: Not a directory", p);
            return 1;
        }
        Err(e) => {
            libmorpheus::eprintln!("rmdir: {}: {}", p, e);
            return 1;
        }
        _ => {}
    }
    match libmorpheus::fs::remove_file(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(_) => {
            libmorpheus::eprintln!("rmdir: {}: Directory not empty", p);
            1
        }
    }
}

pub fn rmdir_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(arg) = args.first() else {
        con.write_colored(fb, "rmdir: missing operand\n", (170, 0, 0));
        return 1;
    };
    let p = path::resolve(cwd, arg);
    match libmorpheus::fs::metadata(&p) {
        Ok(m) if !m.is_dir() => {
            con.write_colored(fb, &format!("rmdir: {}: Not a directory\n", p), (170, 0, 0));
            return 1;
        }
        Err(e) => {
            con.write_colored(fb, &format!("rmdir: {}: {}\n", p, e), (170, 0, 0));
            return 1;
        }
        _ => {}
    }
    match libmorpheus::fs::remove_file(&p) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(_) => {
            con.write_colored(
                fb,
                &format!("rmdir: {}: Directory not empty\n", p),
                (170, 0, 0),
            );
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
    let mut dst = path::resolve(cwd, &args[1]);
    if let Ok(m) = libmorpheus::fs::metadata(&dst) {
        if m.is_dir() {
            let name = path::basename(&src);
            dst.push('/');
            dst.push_str(name);
        }
    }
    if src == dst {
        libmorpheus::eprintln!("mv: '{}' and '{}' are the same file", args[0], args[1]);
        return 1;
    }
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

pub fn mv_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    if args.len() < 2 {
        con.write_colored(fb, "mv: need <src> <dst>\n", (170, 0, 0));
        return 1;
    }
    let src = path::resolve(cwd, &args[0]);
    let mut dst = path::resolve(cwd, &args[1]);
    if let Ok(m) = libmorpheus::fs::metadata(&dst) {
        if m.is_dir() {
            let name = path::basename(&src);
            dst.push('/');
            dst.push_str(name);
        }
    }
    if src == dst {
        con.write_colored(
            fb,
            &format!("mv: '{}' and '{}' are the same file\n", args[0], args[1]),
            (170, 0, 0),
        );
        return 1;
    }
    match libmorpheus::fs::rename_path(&src, &dst) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("mv: {}\n", e), (170, 0, 0));
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
    if let Ok(m) = libmorpheus::fs::metadata(&src) {
        if m.is_dir() {
            libmorpheus::eprintln!("cp: {}: Is a directory (not copied)", src);
            return 1;
        }
    }
    let mut dst = path::resolve(cwd, &args[1]);
    if let Ok(m) = libmorpheus::fs::metadata(&dst) {
        if m.is_dir() {
            let name = path::basename(&src);
            dst.push('/');
            dst.push_str(name);
        }
    }
    if src == dst {
        libmorpheus::eprintln!("cp: '{}' and '{}' are the same file", args[0], args[1]);
        return 1;
    }
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

pub fn cp_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    if args.len() < 2 {
        con.write_colored(fb, "cp: need <src> <dst>\n", (170, 0, 0));
        return 1;
    }
    let src = path::resolve(cwd, &args[0]);
    if let Ok(m) = libmorpheus::fs::metadata(&src) {
        if m.is_dir() {
            con.write_colored(
                fb,
                &format!("cp: {}: Is a directory (not copied)\n", src),
                (170, 0, 0),
            );
            return 1;
        }
    }
    let mut dst = path::resolve(cwd, &args[1]);
    if let Ok(m) = libmorpheus::fs::metadata(&dst) {
        if m.is_dir() {
            let name = path::basename(&src);
            dst.push('/');
            dst.push_str(name);
        }
    }
    if src == dst {
        con.write_colored(
            fb,
            &format!("cp: '{}' and '{}' are the same file\n", args[0], args[1]),
            (170, 0, 0),
        );
        return 1;
    }
    match libmorpheus::fs::copy(&src, &dst) {
        Ok(_) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("cp: {}\n", e), (170, 0, 0));
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
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            libmorpheus::eprintln!("touch: {}: Is a directory", p);
            return 1;
        }
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

pub fn touch_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(arg) = args.first() else {
        con.write_colored(fb, "touch: missing operand\n", (170, 0, 0));
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            con.write_colored(fb, &format!("touch: {}: Is a directory\n", p), (170, 0, 0));
            return 1;
        }
        return 0;
    }
    match libmorpheus::fs::write_bytes(&p, b"") {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("touch: {}: {}\n", p, e), (170, 0, 0));
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
    if p == "/" {
        libmorpheus::println!("  Path: /");
        libmorpheus::println!("  Type: directory");
        libmorpheus::println!("  Size: 0 bytes");
        libmorpheus::println!("   Key: (root)");
        return 0;
    }
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

pub fn stat_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    let Some(arg) = args.first() else {
        con.write_colored(fb, "stat: missing operand\n", (170, 0, 0));
        return 1;
    };
    let p = path::resolve(cwd, arg);
    if p == "/" {
        con.write_str(fb, "  Path: /\n");
        con.write_str(fb, "  Type: directory\n");
        con.write_str(fb, "  Size: 0 bytes\n");
        con.write_str(fb, "   Key: (root)\n");
        return 0;
    }
    match libmorpheus::fs::metadata(&p) {
        Ok(m) => {
            con.write_str(fb, &format!("  Path: {}\n", p));
            con.write_str(
                fb,
                &format!(
                    "  Type: {}\n",
                    if m.is_dir() { "directory" } else { "file" }
                ),
            );
            con.write_str(fb, &format!("  Size: {} bytes\n", m.len()));
            con.write_str(fb, &format!("   Key: 0x{:016x}\n", m.key));
            con.write_str(fb, &format!("   LSN: {} (first: {})\n", m.lsn, m.first_lsn));
            con.write_str(fb, &format!("  Vers: {}\n", m.version_count));
            con.write_str(fb, &format!(" Flags: 0x{:08x}\n", m.flags));
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("stat: {}: {}\n", p, e), (170, 0, 0));
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
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            libmorpheus::eprintln!("write: {}: Is a directory", p);
            return 1;
        }
    }
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

pub fn write_fb(args: &[String], cwd: &str, fb: &Framebuffer, con: &mut Console) -> i32 {
    if args.len() < 2 {
        con.write_colored(fb, "write: need <path> <text>\n", (170, 0, 0));
        return 1;
    }
    let p = path::resolve(cwd, &args[0]);
    if let Ok(m) = libmorpheus::fs::metadata(&p) {
        if m.is_dir() {
            con.write_colored(fb, &format!("write: {}: Is a directory\n", p), (170, 0, 0));
            return 1;
        }
    }
    let text = join_args(&args[1..]);
    match libmorpheus::fs::write_bytes(&p, text.as_bytes()) {
        Ok(()) => {
            super::help::auto_sync();
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("write: {}: {}\n", p, e), (170, 0, 0));
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

pub fn sync_cmd_fb(fb: &Framebuffer, con: &mut Console) -> i32 {
    match libmorpheus::fs::sync() {
        Ok(()) => {
            con.write_str(fb, "synced\n");
            0
        }
        Err(e) => {
            con.write_colored(fb, &format!("sync: {:?}\n", e), (170, 0, 0));
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
