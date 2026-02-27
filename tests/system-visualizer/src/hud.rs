use crate::font;
use crate::state::{ProcessInfo, SystemState};

pub struct Framebuf {
    pub ptr: *mut u32,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
}

impl Framebuf {
    #[inline]
    pub fn put(&self, x: u32, y: u32, color: u32) {
        if x < self.w && y < self.h {
            unsafe {
                *self.ptr.add(y as usize * self.stride as usize + x as usize) = color;
            }
        }
    }

    pub fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let x1 = x.min(self.w);
        let y1 = y.min(self.h);
        let x2 = x.saturating_add(w).min(self.w);
        let y2 = y.saturating_add(h).min(self.h);
        for py in y1..y2 {
            let row = py as usize * self.stride as usize;
            for px in x1..x2 {
                unsafe { *self.ptr.add(row + px as usize) = color; }
            }
        }
    }

    pub fn draw_char(&self, x: u32, y: u32, ch: u8, color: u32) {
        let bits = font::glyph(ch);
        for (row, &b) in bits.iter().enumerate() {
            let py = y + row as u32;
            if py >= self.h { break; }
            let base = py as usize * self.stride as usize;
            for col in 0..5u32 {
                let px = x + col;
                if px >= self.w { break; }
                if (b & (1 << (4 - col))) != 0 {
                    unsafe { *self.ptr.add(base + px as usize) = color; }
                }
            }
        }
    }

    pub fn draw_str(&self, x: u32, y: u32, text: &str, color: u32) {
        let mut cx = x;
        for ch in text.bytes() {
            self.draw_char(cx, y, ch, color);
            cx = cx.saturating_add(font::CELL_W);
        }
    }

    pub fn draw_u32(&self, x: u32, y: u32, val: u32, digits: u32, color: u32) {
        let mut cx = x;
        let mut div = 1u32;
        for _ in 1..digits { div *= 10; }
        for _ in 0..digits {
            let d = ((val / div) % 10) as u8 + b'0';
            self.draw_char(cx, y, d, color);
            cx += font::CELL_W;
            div /= 10;
        }
    }

    pub fn draw_bar(&self, x: u32, y: u32, w: u32, h: u32, pct: u32, fg: u32, bg: u32) {
        self.fill_rect(x, y, w, h, bg);
        let fill = (w * pct.min(100)) / 100;
        self.fill_rect(x, y, fill, h, fg);
    }
}

const COL_BG: u32 = 0x000A0E12;
const COL_PANEL: u32 = 0x00141A22;
const COL_BORDER: u32 = 0x00385060;
const COL_TEXT: u32 = 0x00D8F0E8;
const COL_DIM: u32 = 0x00508878;
const COL_ACCENT: u32 = 0x0020F098;
const COL_WARN: u32 = 0x00FF9030;
const COL_CRIT: u32 = 0x00FF3838;

pub fn draw_system_panel(fb: &Framebuf, state: &SystemState) {
    let px = 8u32;
    let py = 8u32;
    let pw = 200u32;
    let ph = 80u32;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 4;
    fb.draw_str(px + 4, y0, "SYSTEM MONITOR", COL_ACCENT);

    let y1 = y0 + font::CELL_H + 2;
    fb.draw_str(px + 4, y1, "MEM:", COL_DIM);
    fb.draw_u32(px + 30, y1, state.mem_used_mb(), 4, COL_TEXT);
    fb.draw_char(px + 54, y1, b'/', COL_DIM);
    fb.draw_u32(px + 60, y1, state.mem_total_mb(), 4, COL_TEXT);
    fb.draw_str(px + 84, y1, "MB", COL_DIM);

    let mem_pct = if state.total_mem > 0 {
        ((state.total_mem - state.free_mem) * 100 / state.total_mem) as u32
    } else { 0 };
    let bar_color = if mem_pct > 90 { COL_CRIT } else if mem_pct > 70 { COL_WARN } else { COL_ACCENT };
    fb.draw_bar(px + 106, y1, 86, font::GLYPH_H, mem_pct, bar_color, COL_BG);

    let y2 = y1 + font::CELL_H + 1;
    fb.draw_str(px + 4, y2, "PROCS:", COL_DIM);
    fb.draw_u32(px + 42, y2, state.proc_count as u32, 3, COL_TEXT);

    fb.draw_str(px + 72, y2, "UP:", COL_DIM);
    let secs = (state.uptime_ms / 1000) as u32;
    let mins = secs / 60;
    let hrs = mins / 60;
    fb.draw_u32(px + 90, y2, hrs, 2, COL_TEXT);
    fb.draw_char(px + 102, y2, b':', COL_DIM);
    fb.draw_u32(px + 108, y2, mins % 60, 2, COL_TEXT);
    fb.draw_char(px + 120, y2, b':', COL_DIM);
    fb.draw_u32(px + 126, y2, secs % 60, 2, COL_TEXT);

    let y3 = y2 + font::CELL_H + 1;
    fb.draw_str(px + 4, y3, "HEAP:", COL_DIM);
    let heap_kb = (state.heap_used / 1024) as u32;
    fb.draw_u32(px + 36, y3, heap_kb, 5, COL_TEXT);
    fb.draw_str(px + 66, y3, "KB", COL_DIM);
}

pub fn draw_process_panel(
    fb: &Framebuf,
    state: &SystemState,
    selected: Option<usize>,
) {
    let px = 8u32;
    let py = 96u32;
    let pw = 200u32;
    let max_visible = 12usize;
    let row_h = font::CELL_H + 1;
    let header_h = font::CELL_H + 4;
    let ph = header_h + (max_visible as u32 * row_h) + 4;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 3;
    fb.draw_str(px + 4, y0, "PID", COL_DIM);
    fb.draw_str(px + 28, y0, "ST", COL_DIM);
    fb.draw_str(px + 50, y0, "CPU%", COL_DIM);
    fb.draw_str(px + 80, y0, "MEM", COL_DIM);
    fb.draw_str(px + 108, y0, "NAME", COL_DIM);
    hline(fb, px, y0 + font::CELL_H + 1, pw);

    let base_y = y0 + header_h;
    let count = state.proc_count.min(max_visible);

    for i in 0..count {
        let proc = match state.process(i) {
            Some(p) => p,
            None => break,
        };
        let ry = base_y + i as u32 * row_h;
        let is_sel = selected == Some(i);

        if is_sel {
            fb.fill_rect(px + 1, ry, pw - 2, row_h - 1, 0x00283830);
        }

        let c = if is_sel { COL_ACCENT } else { COL_TEXT };
        fb.draw_u32(px + 4, ry, proc.pid, 3, c);

        let st_col = state_color(proc.state);
        fb.draw_str(px + 28, ry, &proc.state_str()[..2], st_col);

        let cpu_int = (proc.cpu_pct as u32).min(99);
        fb.draw_u32(px + 50, ry, cpu_int, 2, if cpu_int > 50 { COL_WARN } else { c });
        fb.draw_char(px + 62, ry, b'%', COL_DIM);

        let mem_display = (proc.mem_kb as u32).min(9999);
        fb.draw_u32(px + 74, ry, mem_display, 4, c);

        let name = proc.name_str();
        let trunc = if name.len() > 12 { &name[..12] } else { name };
        fb.draw_str(px + 108, ry, trunc, c);
    }
}

pub fn draw_fps(fb: &Framebuf, fps: u32, latency_ms: u32) {
    let px = fb.w.saturating_sub(108);
    let py = 8u32;

    fb.fill_rect(px, py, 100, 20, COL_PANEL);
    hline(fb, px, py, 100);

    fb.draw_str(px + 4, py + 4, "FPS:", COL_DIM);
    fb.draw_u32(px + 30, py + 4, fps.min(999), 3, COL_ACCENT);
    fb.draw_str(px + 52, py + 4, "MS:", COL_DIM);
    fb.draw_u32(px + 72, py + 4, latency_ms.min(999), 3, COL_TEXT);
}

pub fn draw_controls(fb: &Framebuf) {
    let px = fb.w.saturating_sub(168);
    let py = fb.h.saturating_sub(72);
    let pw = 160u32;
    let ph = 64u32;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);

    let lines: [&str; 6] = [
        "WASD:ORBIT  ZX:ZOOM",
        "TAB/1-9:SEL  K:KILL",
        "ENTER:FOCUS  F:PIN",
        "ESC:BACK  R:RESET",
        "SPACE:PAUSE  O:SLOW",
        "H:HUD  P:PREV  Q:QUIT",
    ];
    for (i, &line) in lines.iter().enumerate() {
        fb.draw_str(px + 4, py + 4 + i as u32 * (font::CELL_H + 1), line, COL_DIM);
    }
}

pub fn draw_selected_detail(fb: &Framebuf, proc: &ProcessInfo) {
    let px = fb.w.saturating_sub(220);
    let py = 36u32;
    let pw = 212u32;
    let ph = 60u32;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 4;
    fb.draw_str(px + 4, y0, "SELECTED:", COL_ACCENT);
    fb.draw_str(px + 60, y0, proc.name_str(), COL_TEXT);

    let y1 = y0 + font::CELL_H + 1;
    fb.draw_str(px + 4, y1, "PID:", COL_DIM);
    fb.draw_u32(px + 30, y1, proc.pid, 5, COL_TEXT);
    fb.draw_str(px + 64, y1, "PPID:", COL_DIM);
    fb.draw_u32(px + 96, y1, proc.ppid, 5, COL_TEXT);
    fb.draw_str(px + 130, y1, proc.state_str(), state_color(proc.state));

    let y2 = y1 + font::CELL_H + 1;
    fb.draw_str(px + 4, y2, "CPU:", COL_DIM);
    fb.draw_u32(px + 30, y2, proc.cpu_pct as u32, 3, COL_TEXT);
    fb.draw_char(px + 48, y2, b'%', COL_DIM);
    fb.draw_str(px + 60, y2, "MEM:", COL_DIM);
    fb.draw_u32(px + 84, y2, proc.mem_kb as u32, 5, COL_TEXT);
    fb.draw_str(px + 114, y2, "KB", COL_DIM);
    fb.draw_str(px + 130, y2, "PRI:", COL_DIM);
    fb.draw_u32(px + 154, y2, proc.priority, 3, COL_TEXT);
}

pub fn draw_load_graph(fb: &Framebuf, state: &SystemState) {
    let gx = 8u32;
    let gy = fb.h.saturating_sub(110);
    let gw = 120u32;
    let gh = 28u32;

    fb.fill_rect(gx, gy, gw, gh + font::CELL_H + 2, COL_PANEL);
    hline(fb, gx, gy, gw);
    fb.draw_str(gx + 2, gy + 2, "MEM LOAD", COL_DIM);

    let plot_y = gy + font::CELL_H + 2;
    let samples = gw.min(120) as usize;
    for i in 0..samples {
        let pct = state.load_history_sample(samples - 1 - i) as u32;
        let bar_h = (gh * pct) / 100;
        let x = gx + i as u32;
        let col = if pct > 90 { COL_CRIT } else if pct > 70 { COL_WARN } else { COL_ACCENT };
        for dy in 0..bar_h {
            fb.put(x, plot_y + gh - 1 - dy, col);
        }
    }

    let cy = plot_y + gh + 4;
    fb.fill_rect(gx, cy, gw, gh + font::CELL_H + 2, COL_PANEL);
    hline(fb, gx, cy, gw);
    fb.draw_str(gx + 2, cy + 2, "CPU LOAD", COL_DIM);

    let cplot_y = cy + font::CELL_H + 2;
    for i in 0..samples {
        let pct = state.cpu_history_sample(samples - 1 - i) as u32;
        let bar_h = (gh * pct) / 100;
        let x = gx + i as u32;
        let col = if pct > 80 { COL_CRIT } else if pct > 50 { COL_WARN } else { 0x002090E0 };
        for dy in 0..bar_h {
            fb.put(x, cplot_y + gh - 1 - dy, col);
        }
    }
}

pub fn draw_state_bar(fb: &Framebuf, state: &SystemState) {
    let px = 8u32;
    let py = fb.h.saturating_sub(110) - 22;
    let pw = 120u32;
    let ph = 18u32;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);

    let total = state.proc_count.max(1) as u32;
    let rw = (pw * state.ready_count) / total;
    let nw = (pw * state.run_count) / total;
    let bw = (pw * state.blocked_count) / total;

    let y = py + 2;
    fb.fill_rect(px, y, rw, 6, 0x00FFE820);
    fb.fill_rect(px + rw, y, nw, 6, 0x0020FF60);
    fb.fill_rect(px + rw + nw, y, bw, 6, 0x002090FF);

    let ty = y + 8;
    fb.draw_str(px, ty, "R:", COL_DIM);
    fb.draw_u32(px + 12, ty, state.ready_count, 2, 0x00FFE820);
    fb.draw_str(px + 28, ty, "N:", COL_DIM);
    fb.draw_u32(px + 40, ty, state.run_count, 2, 0x0020FF60);
    fb.draw_str(px + 56, ty, "B:", COL_DIM);
    fb.draw_u32(px + 68, ty, state.blocked_count, 2, 0x002090FF);
}

pub fn draw_status_flags(fb: &Framebuf, paused: bool, slow_motion: bool, pinned: bool) {
    let mut cx = fb.w / 2 - 60;
    let y = 4u32;
    if paused {
        fb.draw_str(cx, y, "PAUSED", COL_WARN);
        cx += 48;
    }
    if slow_motion {
        fb.draw_str(cx, y, "SLOW 16x", COL_ACCENT);
        cx += 60;
    }
    if pinned {
        fb.draw_str(cx, y, "PINNED", 0x0060C0FF);
    }
}

fn hline(fb: &Framebuf, x: u32, y: u32, w: u32) {
    for dx in 0..w {
        fb.put(x + dx, y, COL_BORDER);
    }
}

fn state_color(state: u32) -> u32 {
    match state {
        0 => 0x00FFE820,
        1 => 0x0020FF60,
        2 => 0x002090FF,
        3 => 0x00909090,
        4 => 0x00505050,
        _ => COL_TEXT,
    }
}
