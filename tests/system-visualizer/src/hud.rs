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
        let cols = (x2 - x1) as usize;
        if cols == 0 {
            return;
        }
        for py in y1..y2 {
            let off = py as usize * self.stride as usize + x1 as usize;
            let row = unsafe { core::slice::from_raw_parts_mut(self.ptr.add(off), cols) };
            row.fill(color);
        }
    }

    pub fn draw_char(&self, x: u32, y: u32, ch: u8, color: u32) {
        let bits = font::glyph(ch);
        for (row, &b) in bits.iter().enumerate() {
            let py = y + row as u32;
            if py >= self.h {
                break;
            }
            let base = py as usize * self.stride as usize;
            for col in 0..5u32 {
                let px = x + col;
                if px >= self.w {
                    break;
                }
                if (b & (1 << (4 - col))) != 0 {
                    unsafe {
                        *self.ptr.add(base + px as usize) = color;
                    }
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
        for _ in 1..digits {
            div *= 10;
        }
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

const LEFT_COL_X: u32 = 8;
const LEFT_COL_W: u32 = 200;
const TOP_PANEL_Y: u32 = 8;
const TOP_PANEL_H: u32 = 80;

fn per_core_panel_rect(state: &SystemState) -> (u32, u32, u32, u32) {
    let cores = state.per_core_count().max(1) as u32;
    let h = (cores.saturating_mul(10) + 24).clamp(92, 188);
    (LEFT_COL_X, TOP_PANEL_Y + TOP_PANEL_H + 8, LEFT_COL_W, h)
}

pub fn draw_system_panel(fb: &Framebuf, state: &SystemState) {
    let px = LEFT_COL_X;
    let py = TOP_PANEL_Y;
    let pw = LEFT_COL_W;
    let ph = TOP_PANEL_H;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 4;
    fb.draw_str(px + 4, y0, "MORPHEUSX MONITOR", COL_ACCENT);

    let y1 = y0 + font::CELL_H + 2;
    fb.draw_str(px + 4, y1, "MEM:", COL_DIM);
    // Always show in GB when the machine has >= 1 GB total RAM (avoids
    // 2-digit truncation of large MB values like 12288 → "88").
    let (mem_used_display, mem_total_display, unit) = if state.mem_total_mb() >= 1024 {
        let used_gb = state.mem_used_mb() as f32 / 1024.0;
        let total_gb = state.mem_total_mb() as f32 / 1024.0;
        let used_int = used_gb as u32;
        let used_frac = ((used_gb - used_int as f32) * 100.0 + 0.5) as u32;
        let total_int = total_gb as u32;
        let total_frac = ((total_gb - total_int as f32) * 100.0 + 0.5) as u32;
        ((used_int, used_frac), (total_int, total_frac), "GB")
    } else {
        (
            (state.mem_used_mb().min(9999), 0),
            (state.mem_total_mb().min(9999), 0),
            "MB",
        )
    };
    // used: "UU,uu" (2+2 digits for int+frac)
    fb.draw_u32(px + 30, y1, mem_used_display.0, 2, COL_TEXT);
    fb.draw_char(px + 42, y1, b'.', COL_DIM);
    fb.draw_u32(px + 48, y1, mem_used_display.1, 2, COL_TEXT);
    fb.draw_str(px + 60, y1, "/", COL_DIM);
    // total: "TT,tt" (2+2 digits)
    fb.draw_u32(px + 72, y1, mem_total_display.0, 2, COL_TEXT);
    fb.draw_char(px + 84, y1, b'.', COL_DIM);
    fb.draw_u32(px + 90, y1, mem_total_display.1, 2, COL_TEXT);
    fb.draw_str(px + 102, y1, unit, COL_DIM);

    let mem_pct = if state.total_mem > 0 {
        ((state.total_mem - state.free_mem) * 100 / state.total_mem) as u32
    } else {
        0
    };
    let bar_color = if mem_pct > 90 {
        COL_CRIT
    } else if mem_pct > 70 {
        COL_WARN
    } else {
        COL_ACCENT
    };
    fb.draw_bar(px + 120, y1, 76, font::GLYPH_H, mem_pct, bar_color, COL_BG);

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
    // Show kernel heap used / total in KB (heap is 4MB = 4096KB max → 4 digits)
    let heap_used_kb = (state.heap_used / 1024).min(9999) as u32;
    let heap_total_kb = (state.heap_total / 1024).min(9999) as u32;
    fb.draw_u32(px + 36, y3, heap_used_kb, 4, COL_TEXT);
    fb.draw_char(px + 60, y3, b'/', COL_DIM);
    fb.draw_u32(px + 66, y3, heap_total_kb, 4, COL_TEXT);
    fb.draw_str(px + 90, y3, "KB", COL_DIM);
}

pub fn draw_process_panel(fb: &Framebuf, state: &SystemState, selected: Option<usize>) {
    let px = LEFT_COL_X;
    let (_, per_core_y, _, per_core_h) = per_core_panel_rect(state);
    let py = per_core_y + per_core_h + 8;
    let pw = LEFT_COL_W;
    let max_visible = 12usize;
    let row_h = font::CELL_H + 1;
    let header_h = font::CELL_H + 4;
    let ph = header_h + (max_visible as u32 * row_h) + 4;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 3;
    fb.draw_str(px + 4, y0, "PID", COL_DIM);
    fb.draw_str(px + 28, y0, "S", COL_DIM);
    fb.draw_str(px + 40, y0, "CPU%", COL_DIM);
    fb.draw_str(px + 70, y0, "MEM KB", COL_DIM);
    fb.draw_str(px + 112, y0, "NAME", COL_DIM);
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
        fb.draw_char(px + 28, ry, state_char(proc.state), st_col);

        let cpu_int = (proc.cpu_pct as u32).min(100);
        fb.draw_u32(
            px + 40,
            ry,
            cpu_int,
            3,
            if cpu_int > 50 { COL_WARN } else { c },
        );
        fb.draw_char(px + 58, ry, b'%', COL_DIM);

        let mem_display = (proc.mem_kb as u32).min(9999);
        fb.draw_u32(px + 70, ry, mem_display, 4, c);

        let display_name = if proc.pid == 0 {
            "MorpheusX"
        } else {
            proc.name_str()
        };
        let trunc = if display_name.len() > 11 {
            &display_name[..11]
        } else {
            display_name
        };
        let name_color = if proc.pid == 0 { 0x00FFD700 } else { c };
        fb.draw_str(px + 112, ry, trunc, name_color);
    }

    // IDLE row — shown below the process list.
    // idle_pct = fraction of wall-clock time the CPU spent in HLT.
    // Together with per-process cpu_pct values they account for all 100%.
    let idle_row_y = base_y + count as u32 * row_h + 2;
    let idle_int = (state.idle_pct as u32).min(100);
    let idle_col = if idle_int > 80 {
        0x0040FF80
    } else {
        0x00406060
    };
    fb.draw_str(px + 4, idle_row_y, "---", COL_DIM);
    fb.draw_char(px + 28, idle_row_y, b'Z', 0x00406060);
    fb.draw_u32(px + 40, idle_row_y, idle_int, 2, idle_col);
    fb.draw_char(px + 52, idle_row_y, b'%', COL_DIM);
    fb.draw_str(px + 112, idle_row_y, "idle", 0x00406060);
}

pub fn draw_fps(fb: &Framebuf, fps: u32, latency_ms: u32, speed_mult: f32) {
    let px = fb.w.saturating_sub(148);
    let py = 8u32;

    fb.fill_rect(px, py, 140, 20, COL_PANEL);
    hline(fb, px, py, 140);

    fb.draw_str(px + 4, py + 4, "FPS:", COL_DIM);
    fb.draw_u32(px + 30, py + 4, fps.min(999), 3, COL_ACCENT);
    fb.draw_str(px + 52, py + 4, "MS:", COL_DIM);
    fb.draw_u32(px + 70, py + 4, latency_ms.min(999), 3, COL_TEXT);

    // Speed indicator: "SPD:1.0x" — integral and one decimal, e.g. "1.0" "1.5"
    let spd_col = if speed_mult > 1.5 {
        COL_ACCENT
    } else if speed_mult < 1.0 {
        COL_DIM
    } else {
        COL_TEXT
    };
    fb.draw_str(px + 94, py + 4, "SPD:", COL_DIM);
    let int_part = speed_mult as u32;
    let frac_part = ((speed_mult - int_part as f32) * 10.0 + 0.5) as u32;
    fb.draw_u32(px + 118, py + 4, int_part, 1, spd_col);
    fb.draw_char(px + 124, py + 4, b'.', spd_col);
    fb.draw_u32(px + 130, py + 4, frac_part, 1, spd_col);
    fb.draw_char(px + 136, py + 4, b'x', spd_col);
}

pub fn draw_controls(fb: &Framebuf) {
    let px = fb.w.saturating_sub(168);
    let py = fb.h.saturating_sub(72);
    let pw = 160u32;
    let ph = 64u32;

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);

    let lines: [&str; 6] = [
        "WASD:ORBIT    ZX:ZOOM",
        "TAB/1-9:SEL    K:KILL",
        "ENTER:FOCUS     F:PIN",
        "ESC:BACK      R:RESET",
        "SPACE:PAUSE    O:SLOW",
        "^X/Y:SPD  H:HUD  Q:QT",
    ];
    for (i, &line) in lines.iter().enumerate() {
        fb.draw_str(
            px + 4,
            py + 4 + i as u32 * (font::CELL_H + 1),
            line,
            COL_DIM,
        );
    }
}

pub fn draw_selected_detail(fb: &Framebuf, proc: &ProcessInfo) {
    let px = fb.w.saturating_sub(220);
    let py = 36u32;
    let pw = 212u32;
    let ph = if proc.pid == 0 { 72u32 } else { 60u32 };

    fb.fill_rect(px, py, pw, ph, COL_PANEL);
    hline(fb, px, py, pw);
    hline(fb, px, py + ph - 1, pw);

    let y0 = py + 4;
    fb.draw_str(px + 4, y0, "SELECTED:", COL_ACCENT);
    let display_name = if proc.pid == 0 {
        "MORPHEUSX"
    } else {
        proc.name_str()
    };
    let name_color = if proc.pid == 0 { 0x00FFD700 } else { COL_TEXT };
    fb.draw_str(px + 60, y0, display_name, name_color);

    let y1 = y0 + font::CELL_H + 1;
    fb.draw_str(px + 4, y1, "PID:", COL_DIM);
    fb.draw_u32(px + 30, y1, proc.pid, 5, COL_TEXT);
    fb.draw_str(px + 66, y1, "PPID:", COL_DIM);
    fb.draw_u32(px + 96, y1, proc.ppid, 5, COL_TEXT);
    fb.draw_str(px + 136, y1, proc.state_str(), state_color(proc.state));

    let y2 = y1 + font::CELL_H + 1;
    fb.draw_str(px + 4, y2, "CPU:", COL_DIM);
    fb.draw_u32(px + 30, y2, proc.cpu_pct as u32, 3, COL_TEXT);
    fb.draw_char(px + 48, y2, b'%', COL_DIM);
    fb.draw_str(px + 60, y2, "MEM:", COL_DIM);
    fb.draw_u32(px + 84, y2, proc.mem_kb as u32, 5, COL_TEXT);
    fb.draw_str(px + 114, y2, "KB", COL_DIM);
    fb.draw_str(px + 136, y2, "PRI:", COL_DIM);
    fb.draw_u32(px + 160, y2, proc.priority, 3, COL_TEXT);

    if proc.pid == 0 {
        let y3 = y2 + font::CELL_H + 3;
        fb.draw_str(px + 4, y3, "[!] THIS IS THE KERNEL", COL_WARN);
        let y4 = y3 + font::CELL_H + 1;
        fb.draw_str(px + 4, y4, "YOU CAN KILL IT...", COL_CRIT);
        let y5 = y4 + font::CELL_H + 1;
        fb.draw_str(px + 4, y5, "ITS A FEATURE NOT A BUG! :)", COL_DIM);
    }
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
        let col = if pct > 90 {
            COL_CRIT
        } else if pct > 70 {
            COL_WARN
        } else {
            COL_ACCENT
        };
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
        let col = if pct > 80 {
            COL_CRIT
        } else if pct > 50 {
            COL_WARN
        } else {
            0x002090E0
        };
        for dy in 0..bar_h {
            fb.put(x, cplot_y + gh - 1 - dy, col);
        }
    }
}

pub fn draw_per_core_graph(fb: &Framebuf, state: &SystemState) {
    let cores = state.per_core_count();
    if cores == 0 {
        return;
    }

    let (gx, gy, gw, gh) = per_core_panel_rect(state);

    // layered panel shell
    fb.fill_rect(gx, gy, gw, gh, COL_PANEL);
    fb.fill_rect(gx + 1, gy + 1, gw.saturating_sub(2), 1, 0x001E2834);
    fb.fill_rect(gx + 1, gy + gh.saturating_sub(2), gw.saturating_sub(2), 1, 0x000E131A);
    hline(fb, gx, gy, gw);
    fb.draw_str(gx + 2, gy + 2, "CORE UTILIZATION", COL_ACCENT);

    // Header rollup metrics + compact legend.
    let mut sum = 0u32;
    let mut peak = 0u32;
    for core in 0..cores {
        let v = state.per_core_util(core) as u32;
        sum = sum.saturating_add(v);
        if v > peak {
            peak = v;
        }
    }
    let avg = if cores > 0 { sum / cores as u32 } else { 0 };
    fb.draw_str(gx + 96, gy + 2, "A", COL_DIM);
    fb.draw_u32(gx + 102, gy + 2, avg.min(100), 3, COL_TEXT);
    fb.draw_char(gx + 120, gy + 2, b'%', COL_DIM);
    fb.draw_str(gx + 126, gy + 2, "P", COL_DIM);
    fb.draw_u32(gx + 132, gy + 2, peak.min(100), 3, COL_WARN);
    fb.draw_char(gx + 150, gy + 2, b'%', COL_DIM);

    // Compact threshold legend chips: N/W/H
    fb.fill_rect(gx + 156, gy + 3, 4, 4, COL_ACCENT);
    fb.draw_char(gx + 162, gy + 2, b'N', COL_DIM);
    fb.fill_rect(gx + 170, gy + 3, 4, 4, COL_WARN);
    fb.draw_char(gx + 176, gy + 2, b'W', COL_DIM);
    fb.fill_rect(gx + 184, gy + 3, 4, 4, COL_CRIT);
    fb.draw_char(gx + 190, gy + 2, b'H', COL_DIM);

    let content_x = gx + 2;
    let content_y = gy + font::CELL_H + 2;
    let content_w = gw.saturating_sub(4);
    let content_h = gh.saturating_sub(font::CELL_H + 4);

    if content_w < 24 || content_h < 8 {
        return;
    }

    let graph_x = content_x + 40;
    let graph_w = content_w.saturating_sub(40);
    if graph_w < 8 {
        return;
    }

    for core in 0..cores {
        // Evenly partition content so rows exactly fill the panel height.
        let y0 = content_y + (core as u32 * content_h) / cores as u32;
        let y1 = content_y + ((core as u32 + 1) * content_h) / cores as u32;
        let row_h = y1.saturating_sub(y0).max(3);
        if y0 + row_h > gy + gh {
            break;
        }

        let core_idx = core as u32;
        // Idiomatic labels: CPU0, CPU1, ...
        fb.draw_str(content_x, y0, "CPU", COL_DIM);
        fb.draw_u32(content_x + 18, y0, core_idx, 2, COL_TEXT);

        let now_pct = state.per_core_util(core) as u32;
        let pct_col = if now_pct > 85 {
            COL_CRIT
        } else if now_pct > 60 {
            COL_WARN
        } else {
            COL_ACCENT
        };
        fb.draw_u32(content_x + 26, y0, now_pct.min(100), 3, pct_col);
        fb.draw_char(content_x + 44, y0, b'%', COL_DIM);

        // Row background with threshold bands.
        fb.fill_rect(graph_x, y0, graph_w, row_h, COL_BG);
        let mid_50 = y0 + row_h.saturating_sub(1) - ((row_h.saturating_sub(1) * 50) / 100);
        let mid_80 = y0 + row_h.saturating_sub(1) - ((row_h.saturating_sub(1) * 80) / 100);
        if mid_50 > y0 {
            fb.fill_rect(graph_x, y0, graph_w, mid_50 - y0, 0x00101822);
        }
        if mid_80 > y0 {
            fb.fill_rect(graph_x, y0, graph_w, mid_80 - y0, 0x00160F14);
        }

        // Temporal grid lines.
        let mut gx_tick = 0u32;
        while gx_tick < graph_w {
            for dy in 0..row_h {
                fb.put(graph_x + gx_tick, y0 + dy, 0x00141D2A);
            }
            gx_tick = gx_tick.saturating_add(16);
        }

        // Threshold guide lines.
        for x in 0..graph_w {
            fb.put(graph_x + x, mid_50, 0x00307090);
            fb.put(graph_x + x, mid_80, 0x00806030);
        }

        let ramp = core as u32 % 5;
        let (base_col, hi_col) = match ramp {
            0 => (0x002884B0, 0x0038D0FF),
            1 => (0x001F8C66, 0x0028E0A0),
            2 => (0x00986A1C, 0x00F0B030),
            3 => (0x006050B0, 0x00D070FF),
            _ => (0x005A8C2E, 0x00A0F050),
        };

        let mut prev_py = y0 + row_h - 1;
        let mut peak_h = 0u32;
        for age in 0..120usize {
            let p = state.per_core_history_sample(core, age) as u32;
            if p > peak_h {
                peak_h = p;
            }
        }

        for x in 0..graph_w {
            let age = ((graph_w - 1 - x) as usize * 119) / (graph_w - 1) as usize;
            let pct = state.per_core_history_sample(core, age) as u32;
            let h = (row_h.saturating_sub(1) * pct.min(100)) / 100;
            if h == 0 {
                prev_py = y0 + row_h - 1;
                continue;
            }
            let py_top = y0 + row_h - 1 - h;

            // Area fill.
            for dy in 0..h {
                let py = y0 + row_h - 1 - dy;
                let col = if dy + 1 == h || dy + 2 == h { hi_col } else { base_col };
                fb.put(graph_x + x, py, col);
            }

            // Connect neighboring points for a smoother waveform.
            let y_min = py_top.min(prev_py);
            let y_max = py_top.max(prev_py);
            for py in y_min..=y_max {
                fb.put(graph_x + x, py, hi_col);
            }

            // faint glow rails around crest
            if py_top > y0 {
                fb.put(graph_x + x, py_top - 1, 0x00406070);
            }
            if py_top + 1 < y0 + row_h {
                fb.put(graph_x + x, py_top + 1, 0x00304050);
            }

            prev_py = py_top;
        }

        // Peak-hold marker for this row.
        let peak_y = y0 + row_h.saturating_sub(1)
            - ((row_h.saturating_sub(1) * peak_h.min(100)) / 100);
        for x in (graph_x..(graph_x + graph_w)).step_by(6) {
            fb.put(x, peak_y, 0x00F0E070);
        }

        // Animated sweep marker to add temporal direction cue.
        let sweep_x = graph_x + ((state.uptime_ms as u32 / 32) % graph_w.max(1));
        for dy in 0..row_h {
            fb.put(sweep_x, y0 + dy, 0x002C3A4A);
        }

        // Current sample beacon.
        let cur_h = (row_h.saturating_sub(1) * now_pct.min(100)) / 100;
        if cur_h > 0 {
            let cur_y = y0 + row_h - 1 - cur_h;
            let blink = ((state.uptime_ms / 120) & 1) == 0;
            let dot_col = if blink { 0x00F8F8F8 } else { hi_col };
            fb.put(graph_x + graph_w - 1, cur_y, dot_col);
            if cur_y > y0 {
                fb.put(graph_x + graph_w - 1, cur_y - 1, 0x0060A0D0);
            }
            if cur_y + 1 < y0 + row_h {
                fb.put(graph_x + graph_w - 1, cur_y + 1, 0x0060A0D0);
            }
        }

        // One-pixel separator between rows.
        if y1 < content_y + content_h {
            for x in 0..content_w {
                fb.put(content_x + x, y1.saturating_sub(1), 0x00203040);
            }
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

    // Use proc_count as the denominator so the bar always represents
    // the fraction of ALL processes in each state.  The remainder
    // (zombie/dead) is painted dark-gray so the bar is always full.
    let total = state.proc_count.max(1) as u32;
    let rw = (pw * state.ready_count) / total;
    let nw = (pw * state.run_count) / total;
    let bw = (pw * state.blocked_count) / total;
    let used = rw + nw + bw;
    let zw = pw.saturating_sub(used); // zombie/dead remainder

    let y = py + 2;
    fb.fill_rect(px, y, rw, 6, 0x00FFE820); // ready  - yellow
    fb.fill_rect(px + rw, y, nw, 6, 0x0020FF60); // run    - green
    fb.fill_rect(px + rw + nw, y, bw, 6, 0x002090FF); // blocked- blue
    fb.fill_rect(px + rw + nw + bw, y, zw, 6, 0x00303030); // zombie - dark

    let ty = y + 8;
    fb.draw_str(px, ty, "RD:", COL_DIM);
    fb.draw_u32(px + 18, ty, state.ready_count, 2, 0x00FFE820);
    fb.draw_str(px + 34, ty, "RN:", COL_DIM);
    fb.draw_u32(px + 52, ty, state.run_count, 2, 0x0020FF60);
    fb.draw_str(px + 68, ty, "BL:", COL_DIM);
    fb.draw_u32(px + 86, ty, state.blocked_count, 2, 0x002090FF);
}

pub fn draw_status_flags(fb: &Framebuf, paused: bool, slow_motion: bool, pinned: bool) {
    // Compute total pixel width of all active flags so we can center the group.
    const GAP: u32 = 8;
    let mut total_w = 0u32;
    let mut count = 0u32;
    if paused {
        total_w += 6 * font::CELL_W;
        count += 1;
    }
    if slow_motion {
        total_w += 8 * font::CELL_W;
        count += 1;
    }
    if pinned {
        total_w += 6 * font::CELL_W;
        count += 1;
    }
    if count == 0 {
        return;
    }
    total_w += (count - 1) * GAP;

    let mut cx = (fb.w / 2).saturating_sub(total_w / 2);
    let y = 4u32;
    if paused {
        fb.draw_str(cx, y, "PAUSED", COL_WARN);
        cx += 6 * font::CELL_W + GAP;
    }
    if slow_motion {
        fb.draw_str(cx, y, "SLOW 16x", COL_ACCENT);
        cx += 8 * font::CELL_W + GAP;
    }
    if pinned {
        fb.draw_str(cx, y, "PINNED", 0x0060C0FF);
    }
}

fn hline(fb: &Framebuf, x: u32, y: u32, w: u32) {
    if y >= fb.h {
        return;
    }
    let x1 = x.min(fb.w) as usize;
    let x2 = x.saturating_add(w).min(fb.w) as usize;
    let cols = x2 - x1;
    if cols == 0 {
        return;
    }
    let off = y as usize * fb.stride as usize + x1;
    let row = unsafe { core::slice::from_raw_parts_mut(fb.ptr.add(off), cols) };
    row.fill(COL_BORDER);
}

fn state_color(state: u32) -> u32 {
    match state {
        0 => 0x00FFE820, // ready   - yellow
        1 => 0x0020FF60, // running - green
        2 => 0x002090FF, // blocked - blue
        3 => 0x00909090, // zombie  - gray
        4 => 0x00505050, // dead    - dark
        _ => COL_TEXT,
    }
}

fn state_char(state: u32) -> u8 {
    match state {
        0 => b'R', // Ready
        1 => b'N', // ruNning
        2 => b'B', // Blocked
        3 => b'Z', // Zombie
        4 => b'D', // Dead
        _ => b'?',
    }
}
