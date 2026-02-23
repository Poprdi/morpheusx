//! Task Manager Application
//!
//! Displays all live processes, their states, CPU ticks, memory usage and
//! priority.  Allows the user to send SIGTERM / SIGKILL to selected processes.
//!
//! # Controls
//!
//! | Key | Action |
//! |-----|--------|
//! | Up / Down | Move selection |
//! | T | Send SIGTERM (graceful shutdown) |
//! | K | Send SIGKILL (immediate termination) |
//! | S | Send SIGSTOP (pause) |
//! | C | Send SIGCONT (resume) |
//! | R | Force refresh |
//! | Esc | Close window |

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;

// Use top-level hwinit re-exports.
use morpheus_hwinit::serial::puts;
use morpheus_hwinit::{ProcessInfo, ProcessState, Signal, SCHEDULER};

/// Maximum snapshot capacity — matches MAX_PROCESSES (64).
const SNAP_CAP: usize = 64;

use morpheus_ui::app::{App, AppEntry, AppRegistry, AppResult};
use morpheus_ui::canvas::Canvas;
use morpheus_ui::color::Color;
use morpheus_ui::draw::glyph::draw_string;
use morpheus_ui::draw::shapes::{hline, rect_fill};
use morpheus_ui::event::{Event, Key};
use morpheus_ui::font;
use morpheus_ui::theme::Theme;

// ═══════════════════════════════════════════════════════════════════════════
// LAYOUT CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

const HEADER_H: u32 = 40;
const ROW_H: u32 = 16;
const TABLE_PAD: u32 = 8;
const STATUS_H: u32 = 24;

// Column x-offsets (relative to TABLE_PAD)
const COL_PID: u32 = 0;
const COL_NAME: u32 = 40;
const COL_STATE: u32 = 136;
const COL_TICKS: u32 = 200;
const COL_MEM: u32 = 288;
const COL_PRIO: u32 = 368;

// ═══════════════════════════════════════════════════════════════════════════
// STATE DISPLAY HELPERS
// ═══════════════════════════════════════════════════════════════════════════

fn state_name(s: ProcessState) -> &'static str {
    match s {
        ProcessState::Ready => "READY  ",
        ProcessState::Running => "RUNNING",
        ProcessState::Blocked(_) => "BLOCKED",
        ProcessState::Zombie => "ZOMBIE ",
        ProcessState::Terminated => "DEAD   ",
    }
}

fn state_color(s: ProcessState) -> Color {
    match s {
        ProcessState::Running => Color::LIGHT_GREEN,
        ProcessState::Ready => Color::WHITE,
        ProcessState::Blocked(_) => Color::YELLOW,
        ProcessState::Zombie => Color::MAGENTA,
        ProcessState::Terminated => Color::DARK_GRAY,
    }
}

/// Convert a `[u8; 32]` name (null-terminated) to a `&str` slice.
fn name_str(name: &[u8; 32]) -> &str {
    let end = name.iter().position(|&b| b == 0).unwrap_or(32);
    core::str::from_utf8(&name[..end]).unwrap_or("???")
}

/// Format a u64 tick count into a compact string.
fn fmt_ticks(ticks: u64) -> String {
    if ticks < 1_000 {
        format!("{}", ticks)
    } else if ticks < 1_000_000 {
        format!("{}k", ticks / 1_000)
    } else {
        format!("{}M", ticks / 1_000_000)
    }
}

/// Format page count as kilobytes.
fn fmt_pages(pages: u64) -> String {
    let kb = pages * 4; // 4KiB per page
    if kb < 1024 {
        format!("{}k", kb)
    } else {
        format!("{}M", kb / 1024)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// APP STRUCT
// ═══════════════════════════════════════════════════════════════════════════

pub struct TaskManager {
    procs: [ProcessInfo; SNAP_CAP],
    proc_count: usize,
    selected: usize,
    sched_ticks: u32,
    live_count: u32,
    status_msg: String,
    status_ttl: u32,
}

impl TaskManager {
    pub fn new() -> Self {
        puts("[TASKS] new()\n");
        let mut tm = Self {
            procs: [Self::empty_proc_info(); SNAP_CAP],
            proc_count: 0,
            selected: 0,
            sched_ticks: 0,
            live_count: 0,
            status_msg: String::new(),
            status_ttl: 0,
        };
        tm.refresh();
        tm
    }

    const fn empty_proc_info() -> ProcessInfo {
        ProcessInfo {
            pid: 0,
            name: [0u8; 32],
            state: ProcessState::Terminated,
            cpu_ticks: 0,
            pages_alloc: 0,
            priority: 0,
        }
    }

    fn refresh(&mut self) {
        self.proc_count = unsafe { SCHEDULER.snapshot_processes(&mut self.procs) };
        self.sched_ticks = SCHEDULER.tick_count();
        self.live_count = SCHEDULER.live_count();
        if self.proc_count > 0 && self.selected >= self.proc_count {
            self.selected = self.proc_count - 1;
        }
    }

    fn selected_pid(&self) -> Option<u32> {
        (self.proc_count > 0 && self.selected < self.proc_count)
            .then(|| self.procs[self.selected].pid)
    }

    fn send_signal(&mut self, sig: Signal) {
        if let Some(pid) = self.selected_pid() {
            if pid == 0 {
                self.set_status("Cannot signal PID 0 (kernel)");
                return;
            }
            let sig_name = match sig {
                Signal::SIGKILL => "SIGKILL",
                Signal::SIGTERM => "SIGTERM",
                Signal::SIGSTOP => "SIGSTOP",
                Signal::SIGCONT => "SIGCONT",
                Signal::SIGINT => "SIGINT",
                _ => "SIG?",
            };
            unsafe {
                match SCHEDULER.send_signal(pid, sig) {
                    Ok(()) => self.set_status(&format!("Sent {} to PID {}", sig_name, pid)),
                    Err(e) => self.set_status(&format!("Error: {}", e)),
                }
            }
            self.refresh();
        } else {
            self.set_status("No process selected");
        }
    }

    fn set_status(&mut self, msg: &str) {
        self.status_msg.clear();
        self.status_msg.push_str(msg);
        self.status_ttl = 80; // ~0.8 s at 100 Hz
    }

    // ── Layout helper ────────────────────────────────────────────────────

    fn table_top(&self) -> u32 {
        // title bar height + col-label row + 1-px separator
        font::FONT_HEIGHT + 6 + ROW_H + 3
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    fn draw_header(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let hh = font::FONT_HEIGHT + 6;
        rect_fill(canvas, 0, 0, w, hh, theme.title_bg);
        let title = format!(
            "Task Manager  — {} proc  tick #{}",
            self.live_count, self.sched_ticks
        );
        draw_string(
            canvas,
            TABLE_PAD,
            3,
            &title,
            theme.title_fg,
            theme.title_bg,
            &font::FONT_DATA,
        );
    }

    fn draw_col_labels(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let hh = font::FONT_HEIGHT + 6;
        let bg = theme.button_bg;
        let fg = theme.button_fg;
        let w = canvas.width();
        rect_fill(canvas, 0, hh, w, ROW_H, bg);
        let x = TABLE_PAD;
        let y = hh + 1;
        draw_string(canvas, x + COL_PID, y, "PID", fg, bg, &font::FONT_DATA);
        draw_string(canvas, x + COL_NAME, y, "NAME", fg, bg, &font::FONT_DATA);
        draw_string(canvas, x + COL_STATE, y, "STATE", fg, bg, &font::FONT_DATA);
        draw_string(
            canvas,
            x + COL_TICKS,
            y,
            "CPU TICKS",
            fg,
            bg,
            &font::FONT_DATA,
        );
        draw_string(canvas, x + COL_MEM, y, "MEM", fg, bg, &font::FONT_DATA);
        draw_string(canvas, x + COL_PRIO, y, "PRI", fg, bg, &font::FONT_DATA);
        hline(canvas, 0, hh + ROW_H, w, theme.border);
    }

    fn draw_rows(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let t = self.table_top();
        let bot = canvas.height().saturating_sub(STATUS_H);
        let visible_h = bot.saturating_sub(t);
        let max_rows = (visible_h / ROW_H) as usize;

        rect_fill(canvas, 0, t, w, visible_h, theme.bg);

        for i in 0..self.proc_count.min(max_rows) {
            let p = &self.procs[i];
            let row_y = t + i as u32 * ROW_H;
            let is_sel = i == self.selected;

            let (fg, bg) = if is_sel {
                (theme.selection_fg, theme.selection_bg)
            } else if i % 2 == 0 {
                (theme.fg, theme.bg)
            } else {
                (theme.fg, theme.button_bg)
            };

            rect_fill(canvas, 0, row_y, w, ROW_H.saturating_sub(1), bg);

            let x = TABLE_PAD;
            let y = row_y + 1;

            draw_string(
                canvas,
                x + COL_PID,
                y,
                &format!("{}", p.pid),
                fg,
                bg,
                &font::FONT_DATA,
            );

            let name = name_str(&p.name);
            let trunc = if name.len() > 11 { &name[..11] } else { name };
            draw_string(canvas, x + COL_NAME, y, trunc, fg, bg, &font::FONT_DATA);

            let sc = if is_sel { fg } else { state_color(p.state) };
            draw_string(
                canvas,
                x + COL_STATE,
                y,
                state_name(p.state),
                sc,
                bg,
                &font::FONT_DATA,
            );

            draw_string(
                canvas,
                x + COL_TICKS,
                y,
                &fmt_ticks(p.cpu_ticks),
                fg,
                bg,
                &font::FONT_DATA,
            );
            draw_string(
                canvas,
                x + COL_MEM,
                y,
                &fmt_pages(p.pages_alloc),
                fg,
                bg,
                &font::FONT_DATA,
            );
            draw_string(
                canvas,
                x + COL_PRIO,
                y,
                &format!("{}", p.priority),
                fg,
                bg,
                &font::FONT_DATA,
            );
        }

        if self.proc_count == 0 {
            draw_string(
                canvas,
                TABLE_PAD,
                t + 4,
                "No processes.",
                Color::DARK_GRAY,
                theme.bg,
                &font::FONT_DATA,
            );
        }
    }

    fn draw_status_bar(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        let y = h.saturating_sub(STATUS_H);
        rect_fill(canvas, 0, y, w, STATUS_H, theme.button_bg);
        hline(canvas, 0, y, w, theme.border);

        let (msg, fg): (&str, Color) = if self.status_ttl > 0 {
            (&self.status_msg, Color::YELLOW)
        } else {
            (
                "Up/Dn Sel  T=SIGTERM  K=SIGKILL  S=SIGSTOP  C=SIGCONT  R=Refresh  Esc=Close",
                Color::DARK_GRAY,
            )
        };
        draw_string(
            canvas,
            TABLE_PAD,
            y + 3,
            msg,
            fg,
            theme.button_bg,
            &font::FONT_DATA,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// APP TRAIT IMPL
// ═══════════════════════════════════════════════════════════════════════════

impl App for TaskManager {
    fn title(&self) -> &str {
        "Task Manager"
    }

    fn default_size(&self) -> (u32, u32) {
        (480, 340)
    }

    fn init(&mut self, canvas: &mut dyn Canvas, theme: &Theme) {
        canvas.clear(theme.bg);
        self.refresh();
        puts("[TASKS] init done\n");
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        canvas.clear(theme.bg);
        self.draw_header(canvas, theme);
        self.draw_col_labels(canvas, theme);
        self.draw_rows(canvas, theme);
        self.draw_status_bar(canvas, theme);
    }

    fn handle_event(&mut self, event: &Event) -> AppResult {
        match event {
            Event::Tick => {
                self.refresh();
                // Count down status message TTL.
                if self.status_ttl > 0 {
                    self.status_ttl -= 1;
                }
                AppResult::Redraw
            }

            Event::KeyPress(ke) => match ke.key {
                Key::Escape => AppResult::Close,

                Key::Up => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                    AppResult::Redraw
                }

                Key::Down => {
                    if self.proc_count > 0 && self.selected + 1 < self.proc_count {
                        self.selected += 1;
                    }
                    AppResult::Redraw
                }

                Key::Char('k') | Key::Char('K') => {
                    self.send_signal(Signal::SIGKILL);
                    AppResult::Redraw
                }

                Key::Char('t') | Key::Char('T') => {
                    self.send_signal(Signal::SIGTERM);
                    AppResult::Redraw
                }

                Key::Char('s') | Key::Char('S') => {
                    self.send_signal(Signal::SIGSTOP);
                    AppResult::Redraw
                }

                Key::Char('c') | Key::Char('C') => {
                    self.send_signal(Signal::SIGCONT);
                    AppResult::Redraw
                }

                Key::Char('r') | Key::Char('R') => {
                    self.refresh();
                    AppResult::Redraw
                }

                _ => AppResult::Continue,
            },

            _ => AppResult::Continue,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// REGISTRATION
// ═══════════════════════════════════════════════════════════════════════════

pub fn register(registry: &mut AppRegistry) {
    registry.register(AppEntry {
        name: "tasks",
        title: "Task Manager",
        default_size: (480, 340),
        create: || Box::new(TaskManager::new()),
    });
}
