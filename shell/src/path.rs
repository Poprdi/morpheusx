extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use libmorpheus::fs;

pub fn resolve(cwd: &str, input: &str) -> String {
    let raw = if input.starts_with('/') {
        String::from(input)
    } else {
        let mut full = String::with_capacity(cwd.len() + 1 + input.len());
        full.push_str(cwd);
        if !cwd.ends_with('/') {
            full.push('/');
        }
        full.push_str(input);
        full
    };
    normalize(&raw)
}

pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut out = String::with_capacity(path.len());
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    out
}

pub fn which(name: &str, cwd: &str) -> Option<String> {
    if name.contains('/') {
        let abs = resolve(cwd, name);
        return if file_exists(&abs) { Some(abs) } else { None };
    }

    const SEARCH: &[&str] = &["/bin/"];

    for dir in SEARCH {
        let mut path = String::with_capacity(dir.len() + name.len());
        path.push_str(dir);
        path.push_str(name);
        if file_exists(&path) {
            return Some(path);
        }
    }
    None
}

fn file_exists(path: &str) -> bool {
    match fs::metadata(path) {
        Ok(m) => m.is_file(),
        Err(_) => false,
    }
}

pub fn basename(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[i + 1..],
        None => path,
    }
}
