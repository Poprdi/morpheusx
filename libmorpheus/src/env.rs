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

pub fn current_dir() -> error::Result<String> {
    let mut buf = [0u8; 512];
    let n = crate::fs::getcwd(&mut buf).map_err(Error::from_raw)?;
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("/");
    Ok(String::from(s))
}

pub fn set_current_dir(path: &str) -> error::Result<()> {
    crate::fs::chdir(path).map_err(Error::from_raw)
}
