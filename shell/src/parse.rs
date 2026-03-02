extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug)]
pub struct Pipeline {
    pub commands: Vec<SimpleCommand>,
}

#[derive(Debug)]
pub struct SimpleCommand {
    pub argv: Vec<String>,
    pub stdin_file: Option<String>,
    pub stdout_file: Option<Redirect>,
}

#[derive(Debug)]
pub struct Redirect {
    pub path: String,
    pub append: bool,
}

pub fn parse(line: &str) -> Option<Pipeline> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let segments = split_pipe(trimmed);
    let mut commands = Vec::with_capacity(segments.len());

    for seg in segments {
        let tokens = tokenize(seg);
        if tokens.is_empty() {
            continue;
        }
        match build_command(tokens) {
            Some(cmd) => commands.push(cmd),
            None => continue,
        }
    }

    if commands.is_empty() {
        return None;
    }
    Some(Pipeline { commands })
}

fn split_pipe(s: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        if escape {
            escape = false;
            continue;
        }
        match bytes[i] {
            b'\\' if !in_single => escape = true,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'|' if !in_single && !in_double => {
                segments.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(&s[start..]);
    segments
}

fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    for ch in s.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }

        match ch {
            '\\' if !in_single => {
                escape = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(core::mem::replace(&mut current, String::new()));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn build_command(tokens: Vec<String>) -> Option<SimpleCommand> {
    let mut argv = Vec::new();
    let mut stdin_file = None;
    let mut stdout_file = None;
    let mut iter = tokens.into_iter();

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "<" => {
                stdin_file = iter.next();
            }
            ">" => {
                if let Some(path) = iter.next() {
                    stdout_file = Some(Redirect {
                        path,
                        append: false,
                    });
                }
            }
            ">>" => {
                if let Some(path) = iter.next() {
                    stdout_file = Some(Redirect { path, append: true });
                }
            }
            _ => {
                if tok.starts_with('>') {
                    let rest = &tok[1..];
                    let (append, path_part) = if rest.starts_with('>') {
                        (true, &rest[1..])
                    } else {
                        (false, rest)
                    };
                    let path = if path_part.is_empty() {
                        match iter.next() {
                            Some(p) => p,
                            None => continue,
                        }
                    } else {
                        String::from(path_part)
                    };
                    stdout_file = Some(Redirect { path, append });
                } else if tok.starts_with('<') {
                    let rest = &tok[1..];
                    stdin_file = if rest.is_empty() {
                        iter.next()
                    } else {
                        Some(String::from(rest))
                    };
                } else {
                    argv.push(tok);
                }
            }
        }
    }

    if argv.is_empty() {
        return None;
    }
    Some(SimpleCommand {
        argv,
        stdin_file,
        stdout_file,
    })
}
