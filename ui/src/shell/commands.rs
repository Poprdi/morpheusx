use alloc::string::String;
use alloc::format;

pub enum CommandResult {
    Output(String),
    Clear,
    OpenApp(String),
    CloseWindow(u32),
    ListWindows,
    Exit,
    Unknown(String),
}

pub fn execute(input: &str, _window_ids: &[u32]) -> CommandResult {
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
        "close" => {
            if let Some(id) = parse_u32(arg) {
                CommandResult::CloseWindow(id)
            } else {
                CommandResult::Output(String::from("Usage: close <window-id>"))
            }
        }
        "list" | "windows" => CommandResult::ListWindows,
        _ => CommandResult::Unknown(String::from(cmd)),
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
         \x20 help          - Show this help\n\
         \x20 clear         - Clear output\n\
         \x20 open <app>    - Open an application\n\
         \x20 close <id>    - Close window by ID\n\
         \x20 list          - List open windows\n\
         \x20 exit          - Return to firmware"
    )
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(result)
}
