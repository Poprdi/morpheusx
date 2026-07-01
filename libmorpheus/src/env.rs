//! Process environment: argv and cwd.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{self, Error};

pub struct Args {
    args: Vec<String>,
    pos: usize,
}

impl Iterator for Args {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.args.len() {
            let arg = self.args[self.pos].clone();
            self.pos += 1;
            Some(arg)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.args.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Args {}

/// Command-line arguments; argv[0] is typically the program path.
pub fn args() -> Args {
    let mut buf = [0u8; 4096];
    let n = crate::process::getargs(&mut buf);
    if n == 0 {
        return Args {
            args: Vec::new(),
            pos: 0,
        };
    }

    let mut strs: [&str; 64] = [""; 64];
    let count = crate::process::parse_args(&buf[..n], &mut strs);

    let mut args = Vec::with_capacity(count);
    for s in &strs[..count] {
        args.push(String::from(*s));
    }

    Args { args, pos: 0 }
}

pub fn args_vec() -> Vec<String> {
    args().collect()
}

/// Snapshot the initial environ block (NUL-separated `KEY=VALUE`) via SYS_GETENV.
fn env_block() -> Vec<u8> {
    let total = unsafe { crate::raw::sys_getenv(0, 0) };
    if crate::is_error(total) || total == 0 {
        return Vec::new();
    }
    let mut buf = alloc::vec![0u8; total as usize];
    let n = unsafe { crate::raw::sys_getenv(buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(n) {
        return Vec::new();
    }
    buf.truncate((n as usize).min(buf.len()));
    buf
}

/// Value of environment variable `key`; `ENOENT` if unset.
pub fn var(key: &str) -> error::Result<String> {
    let block = env_block();
    for rec in block.split(|&b| b == 0) {
        if rec.is_empty() {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(rec) {
            if let Some((k, v)) = s.split_once('=') {
                if k == key {
                    return Ok(String::from(v));
                }
            }
        }
    }
    Err(Error::from_raw(morpheus_foundation::errno::ENOENT))
}

/// All `(key, value)` pairs from the initial environ block.
pub fn vars() -> Vec<(String, String)> {
    let block = env_block();
    let mut out = Vec::new();
    for rec in block.split(|&b| b == 0) {
        if rec.is_empty() {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(rec) {
            if let Some((k, v)) = s.split_once('=') {
                out.push((String::from(k), String::from(v)));
            }
        }
    }
    out
}

pub fn current_dir() -> error::Result<String> {
    let mut buf = [0u8; 512];
    let n = crate::fs::getcwd(&mut buf).map_err(Error::from_raw)?;
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("/");
    Ok(String::from(s))
}

pub fn set_current_dir(path: &str) -> error::Result<()> {
    crate::fs::chdir(path).map_err(Error::from_raw)
}
