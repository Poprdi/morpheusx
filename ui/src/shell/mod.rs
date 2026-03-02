pub mod commands;
pub mod ring_buffer;

use crate::canvas::Canvas;
use crate::draw::glyph::draw_string;
use crate::draw::shapes::{hline, rect_fill};
use crate::event::{Event, Key, KeyEvent};
use crate::font;
use crate::font::FONT_DATA;
use crate::theme::Theme;
use crate::widget::text_input::TextInput;
use crate::widget::Widget;
use alloc::format;
use alloc::string::String;

use self::commands::{CommandResult, FsOp};
use self::ring_buffer::RingBuffer;

const OUTPUT_CAPACITY: usize = 4096;
const INPUT_MAX: usize = 256;
const PROMPT: &str = "morpheus> ";

pub enum ShellAction {
    None,
    OpenApp(String),
    CloseWindow(u32),
    ListWindows,
    SpawnProcess(String),
    FsCommand(FsOp),
    Exit,
}

pub struct Shell {
    output: RingBuffer<String>,
    input: TextInput,
    scroll_top: usize,
    cwd: String,
    prompt: String,
    /// Optional hook called for every line of output (e.g. serial echo).
    /// The UI crate has no deps, so the bootloader sets this to a serial puts fn.
    echo_hook: Option<fn(&str)>,
}

impl Shell {
    pub fn new() -> Self {
        let mut s = Self {
            output: RingBuffer::new(OUTPUT_CAPACITY),
            input: TextInput::new(INPUT_MAX),
            scroll_top: 0,
            cwd: String::from("/"),
            prompt: String::from("morpheus:/> "),
            echo_hook: None,
        };
        s.output.push(String::from("MorpheusX Shell v0.1"));
        s.output.push(String::from("Type 'help' for commands."));
        s.output.push(String::new());
        s.input.set_focused(true);
        s
    }

    pub fn push_output(&mut self, text: &str) {
        for line in text.split('\n') {
            if let Some(echo) = self.echo_hook {
                echo(line);
                echo("\n");
            }
            self.output.push(String::from(line));
        }
        self.scroll_to_bottom_internal();
    }

    /// Register a hook that receives every line of shell output.
    /// Intended for serial console mirroring.
    pub fn set_echo_hook(&mut self, hook: fn(&str)) {
        self.echo_hook = Some(hook);
    }

    /// Current working directory.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Update the working directory.  Called by the desktop after VFS verification.
    pub fn set_cwd(&mut self, path: &str) {
        self.cwd.clear();
        self.cwd.push_str(path);
        self.prompt.clear();
        self.prompt.push_str("morpheus:");
        self.prompt.push_str(path);
        self.prompt.push_str("> ");
    }

    pub fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();

        canvas.clear(theme.bg);

        let input_h = font::FONT_HEIGHT + 4;
        let output_h = h.saturating_sub(input_h + 1);
        let vis_lines = (output_h / font::FONT_HEIGHT) as usize;
        let text_cols = (w / font::FONT_WIDTH) as usize;

        for i in 0..vis_lines {
            let line_idx = self.scroll_top + i;
            if let Some(line) = self.output.get(line_idx) {
                let y = i as u32 * font::FONT_HEIGHT;
                let display: &str = if line.len() > text_cols {
                    &line[..text_cols]
                } else {
                    line
                };
                draw_string(canvas, 0, y, display, theme.fg, theme.bg, &FONT_DATA);
            }
        }

        let sep_y = output_h;
        hline(canvas, 0, sep_y, w, theme.border);

        let input_y = sep_y + 1;
        rect_fill(canvas, 0, input_y, w, input_h, theme.input_bg);

        let prompt_w = self.prompt.len() as u32 * font::FONT_WIDTH;
        draw_string(
            canvas,
            0,
            input_y + 2,
            &self.prompt,
            theme.accent,
            theme.input_bg,
            &FONT_DATA,
        );

        let input_text = self.input.text();
        let display: &str = if input_text.len() > text_cols.saturating_sub(self.prompt.len()) {
            let max = text_cols.saturating_sub(self.prompt.len());
            &input_text[..max]
        } else {
            input_text
        };
        draw_string(
            canvas,
            prompt_w,
            input_y + 2,
            display,
            theme.input_fg,
            theme.input_bg,
            &FONT_DATA,
        );

        let cursor_x = prompt_w + display.len() as u32 * font::FONT_WIDTH;
        canvas.fill_rect(
            cursor_x,
            input_y + 2,
            1,
            font::FONT_HEIGHT,
            theme.input_cursor,
        );
    }

    pub fn handle_event(&mut self, event: &Event, window_ids: &[u32]) -> ShellAction {
        if let Event::KeyPress(KeyEvent { key, .. }) = event {
            match key {
                Key::Enter => {
                    let text = self.input.take_text();
                    let cmd_line = format!("{}{}", self.prompt, text);
                    if let Some(echo) = self.echo_hook {
                        echo(&cmd_line);
                        echo("\n");
                    }
                    self.output.push(cmd_line);

                    let result = commands::execute(&text, window_ids, &self.cwd);
                    let action = match result {
                        CommandResult::Output(s) => {
                            if !s.is_empty() {
                                self.push_output(&s);
                            }
                            ShellAction::None
                        }
                        CommandResult::Clear => {
                            self.output.clear();
                            ShellAction::None
                        }
                        CommandResult::OpenApp(name) => ShellAction::OpenApp(name),
                        CommandResult::CloseWindow(id) => ShellAction::CloseWindow(id),
                        CommandResult::ListWindows => ShellAction::ListWindows,
                        CommandResult::SpawnProcess(name) => ShellAction::SpawnProcess(name),
                        CommandResult::FsCommand(op) => ShellAction::FsCommand(op),
                        CommandResult::Exit => ShellAction::Exit,
                        CommandResult::Unknown(cmd) => {
                            self.push_output(&format!("Unknown command: {}", cmd));
                            ShellAction::None
                        }
                    };

                    self.scroll_to_bottom_internal();
                    return action;
                }
                Key::PageUp => {
                    self.scroll_top = self.scroll_top.saturating_sub(10);
                    return ShellAction::None;
                }
                Key::PageDown => {
                    self.scroll_to_bottom_internal();
                    return ShellAction::None;
                }
                _ => {
                    self.input.handle_event(event);
                }
            }
        }
        ShellAction::None
    }

    fn scroll_to_bottom_internal(&mut self) {
        let total = self.output.len();
        let vis = 20usize;
        if total > vis {
            self.scroll_top = total - vis;
        } else {
            self.scroll_top = 0;
        }
    }
}
