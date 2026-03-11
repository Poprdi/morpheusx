// mist shore — display baseline controls.
// framebuffer introspection, pixel format confirmation, stride/resolution readout.
// read-only telemetry mostly — this is the calmest chamber.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{Route, SettingsApp};
use crate::widgets;

use libmorpheus::hw;

const FIELD_REFRESH: usize = 0;
const FIELD_COUNT: usize = 1;

pub struct MistChamber {
    pub fb_width: u32,
    pub fb_height: u32,
    pub fb_stride: u32,
    pub fb_format: u32,
    pub fb_size: u64,
    pub fb_base: u64,
}

impl MistChamber {
    pub fn new() -> Self {
        Self {
            fb_width: 0,
            fb_height: 0,
            fb_stride: 0,
            fb_format: 0,
            fb_size: 0,
            fb_base: 0,
        }
    }

    pub fn refresh(&mut self) {
        if let Ok(info) = hw::fb_info() {
            self.fb_width = info.width;
            self.fb_height = info.height;
            self.fb_stride = info.stride;
            self.fb_format = info.format;
            self.fb_size = info.size;
            self.fb_base = info.base;
        }
    }

    pub fn widget_count(&self) -> usize {
        FIELD_COUNT
    }

    pub fn activate(&mut self, idx: usize, app: &mut SettingsApp) {
        match idx {
            FIELD_REFRESH => {
                self.refresh();
                app.set_status("Display info refreshed", false);
            }
            _ => {}
        }
    }

    pub fn apply(&mut self) {
        // mist shore is read-only telemetry. nothing to apply.
    }

    pub fn revert(&mut self) {}
    pub fn restore_defaults(&mut self) {}

    pub fn handle_key(&mut self, _scancode: u8, _app: &mut SettingsApp) {}

    pub fn handle_click(&mut self, _px: i32, py: i32, app: &mut SettingsApp) {
        let row_h = (widgets::FONT_H + 8) as i32;
        let idx = ((py - 40) / row_h).max(0) as usize;
        if idx < FIELD_COUNT {
            app.pane_focus = idx;
            self.activate(idx, app);
        }
    }
}

pub fn render(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;
    let mist = &app.mist;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;

    layout::draw_section(app, px, cy, "Framebuffer");
    cy += widgets::FONT_H + 4;

    // resolution
    let mut buf = [0u8; 32];
    let mut res = [0u8; 24];
    let mut ri = 0;
    let wn = widgets::u64_to_str(mist.fb_width as u64, &mut buf);
    res[ri..ri + wn].copy_from_slice(&buf[..wn]);
    ri += wn;
    res[ri] = b'x';
    ri += 1;
    let hn = widgets::u64_to_str(mist.fb_height as u64, &mut buf);
    res[ri..ri + hn].copy_from_slice(&buf[..hn]);
    ri += hn;
    let res_str = core::str::from_utf8(&res[..ri]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Resolution:", res_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // stride
    let n = widgets::u64_to_str(mist.fb_stride as u64, &mut buf);
    let stride_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Stride (bytes):", stride_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // stride in pixels
    let stride_px = mist.fb_stride / 4;
    let n = widgets::u64_to_str(stride_px as u64, &mut buf);
    let spx_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Stride (pixels):", spx_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // pixel format
    let fmt_str = match mist.fb_format {
        0 => "RGBX (0)",
        1 => "BGRX (1)",
        2 => "BitMask (2)",
        3 => "BltOnly (3)",
        _ => "Unknown",
    };
    layout::draw_kv(app, px, cy, "Pixel Format:", fmt_str, t.immutable);
    cy += widgets::FONT_H + 2;

    // fb size
    let n = widgets::format_bytes(mist.fb_size, &mut buf);
    let size_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "FB Size:", size_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // base address
    let mut hex_buf = [0u8; 18];
    let hex_len = format_hex(mist.fb_base, &mut hex_buf);
    let hex_str = core::str::from_utf8(&hex_buf[..hex_len]).unwrap_or("0x???");
    layout::draw_kv(app, px, cy, "Base Addr:", hex_str, t.immutable);
    cy += widgets::FONT_H + 8;

    // pixel math section
    layout::draw_section(app, px, cy, "Pixel Math");
    cy += widgets::FONT_H + 4;

    let bpp = 4u32;
    let n = widgets::u64_to_str(bpp as u64, &mut buf);
    let bpp_str = core::str::from_utf8(&buf[..n]).unwrap_or("4");
    layout::draw_kv(app, px, cy, "Bytes/Pixel:", bpp_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // total pixels
    let total_px = mist.fb_width as u64 * mist.fb_height as u64;
    let n = widgets::u64_to_str(total_px, &mut buf);
    let tpx_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Total Pixels:", tpx_str, t.telemetry);
    cy += widgets::FONT_H + 2;

    // scanline padding
    let pad = mist.fb_stride.saturating_sub(mist.fb_width * bpp);
    let pad_label = if pad == 0 { "None" } else { "Present" };
    let pad_color = if pad == 0 { t.success } else { t.warning };
    layout::draw_kv(app, px, cy, "Scanline Pad:", pad_label, pad_color);
    cy += widgets::FONT_H + 2;

    if pad > 0 {
        let n = widgets::u64_to_str(pad as u64, &mut buf);
        let pad_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
        layout::draw_kv(app, px, cy, "  Pad Bytes:", pad_str, t.warning);
        cy += widgets::FONT_H + 2;
    }

    cy += 8;

    // packing reminder
    layout::draw_section(app, px, cy, "Packing Reference");
    cy += widgets::FONT_H + 4;

    let packing = match mist.fb_format {
        1 => "BGRX: b | (g<<8) | (r<<16) | (0xFF<<24)",
        0 => "RGBX: r | (g<<8) | (b<<16) | (0xFF<<24)",
        _ => "(non-standard format)",
    };
    widgets::draw_str(s, st, px, cy, packing, t.immutable, t.substrate, w, h);
    cy += widgets::FONT_H + 2;
    widgets::draw_str(s, st, px, cy, "addr = base + (y * stride) + (x * 4)", t.glyph_dim, t.substrate, w, h);
    cy += widgets::FONT_H + 12;

    layout::draw_button_row(app, px, cy, "Refresh Display Info", FIELD_REFRESH, t.glyph);
}

fn format_hex(val: u64, buf: &mut [u8; 18]) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    buf[0] = b'0';
    buf[1] = b'x';
    let mut i = 2;
    // skip leading zeros but keep at least one digit
    let mut started = false;
    for shift in (0..16).rev() {
        let nib = ((val >> (shift * 4)) & 0xF) as usize;
        if nib != 0 || started || shift == 0 {
            buf[i] = HEX[nib];
            i += 1;
            started = true;
        }
    }
    i
}
