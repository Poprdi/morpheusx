//! Blue Screen of Death — MorpheusX crash screen.
//!
//! Renders a full-screen crash display directly to the framebuffer with:
//!   - Dark-tinted Morpheus background image (RLE-compressed thumbnail scaled up)
//!   - Sad face emoticon
//!   - Detailed crash information (vector, error code, registers)
//!   - Panic location if available
//!
//! **Zero allocation** — this module must work even when the heap is dead.

#[path = "bsod_bg_data.rs"]
mod bsod_bg_data;
#[path = "bsod_bg_data_v1.rs"]
#[allow(dead_code)]
mod bsod_bg_data_v1;

use morpheus_display::font::{get_glyph_or_space, FONT_HEIGHT, FONT_WIDTH};
use morpheus_hwinit::serial::puts;

use crate::baremetal;

// ═══════════════════════════════════════════════════════════════════════════
// FRAMEBUFFER PRIMITIVES (no alloc, direct pixel writes)
// ═══════════════════════════════════════════════════════════════════════════

/// Write a single pixel to the framebuffer.
///
/// `stride` is pixels_per_scan_line (from GOP), NOT bytes.
/// `format`: 0=RGBX, 1=BGRX.
#[inline(always)]
unsafe fn put_pixel(fb: *mut u32, stride: u32, x: u32, y: u32, rgb: u32) {
    let offset = (y as usize) * (stride as usize) + (x as usize);
    fb.add(offset).write_volatile(rgb);
}

/// Fill a rectangle with a solid color.
unsafe fn fill_rect(fb: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, rgb: u32) {
    for row in y..y + h {
        for col in x..x + w {
            put_pixel(fb, stride, col, row, rgb);
        }
    }
}

/// Convert 0x00RRGGBB → framebuffer pixel value according to format.
#[inline(always)]
fn to_fb_pixel(rgb: u32, format: u32) -> u32 {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    if format == 0 {
        // RGBX: R in low byte
        (r) | (g << 8) | (b << 16)
    } else {
        // BGRX (most common): B in low byte
        (b) | (g << 8) | (r << 16)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BACKGROUND IMAGE RENDERING
// ═══════════════════════════════════════════════════════════════════════════

/// Expand a palette index (0x0RGB, 4-bit/channel) to 0x00RRGGBB.
#[inline(always)]
fn palette_to_rgb(color: u16) -> u32 {
    let r4 = ((color >> 8) & 0xF) as u32;
    let g4 = ((color >> 4) & 0xF) as u32;
    let b4 = (color & 0xF) as u32;
    // Expand 4-bit to 8-bit: 0xN → 0xNN
    let r = (r4 << 4) | r4;
    let g = (g4 << 4) | g4;
    let b = (b4 << 4) | b4;
    (r << 16) | (g << 8) | b
}

/// Decode palette+RLE background image, scaled to fill the screen.
/// Uses nearest-neighbor scaling from thumbnail to screen resolution.
///
/// v2 format: palette `[u16; N]` + RLE `[u8; M]` where each pair is
/// `(count: u8, palette_index: u8)`.
unsafe fn draw_background(fb: *mut u32, stride: u32, width: u32, height: u32, format: u32) {
    let src_w = bsod_bg_data::BG_WIDTH;
    let src_h = bsod_bg_data::BG_HEIGHT;
    let palette = &bsod_bg_data::BG_PALETTE;
    let rle = &bsod_bg_data::BG_RLE_DATA;
    let rle_len = rle.len();

    // RLE cursor state
    let mut rle_byte_idx: usize = 0; // byte index into BG_RLE_DATA (steps by 2)
    let mut rle_pos: u8 = 0;         // position within current run
    let mut src_pixel_idx: usize = 0; // linear source pixel index

    // Helper: read current run (count, palette_index) from RLE
    #[inline(always)]
    fn rle_run(rle: &[u8], byte_idx: usize) -> (u8, u8) {
        if byte_idx + 1 < rle.len() {
            (rle[byte_idx], rle[byte_idx + 1])
        } else {
            (0, 0)
        }
    }

    for oy in 0..height {
        let sy = (oy as u64 * src_h as u64 / height as u64) as u32;
        let target_row_start = (sy * src_w) as usize;
        let target_row_end = target_row_start + src_w as usize;

        // Advance RLE cursor to start of this source row
        while src_pixel_idx < target_row_start && rle_byte_idx < rle_len {
            let (count, _idx) = rle_run(rle, rle_byte_idx);
            let remaining = count - rle_pos;
            let skip = target_row_start - src_pixel_idx;
            if skip >= remaining as usize {
                src_pixel_idx += remaining as usize;
                rle_byte_idx += 2;
                rle_pos = 0;
            } else {
                rle_pos += skip as u8;
                src_pixel_idx += skip;
            }
        }

        // Extract source pixels for this row (on the stack — 120 × 4 = 480 bytes)
        let mut row_buf = [0u32; 120]; // BG_WIDTH = 120
        let mut ri = 0usize;
        let mut tmp_byte_idx = rle_byte_idx;
        let mut tmp_pos = rle_pos;

        while ri < src_w as usize && tmp_byte_idx < rle_len {
            let (count, pal_idx) = rle_run(rle, tmp_byte_idx);
            let remaining = count - tmp_pos;
            let need = (src_w as usize) - ri;
            let take = need.min(remaining as usize);
            let rgb = if (pal_idx as usize) < palette.len() {
                palette_to_rgb(palette[pal_idx as usize])
            } else {
                0
            };
            for j in 0..take {
                row_buf[ri + j] = rgb;
            }
            ri += take;
            tmp_pos += take as u8;
            if tmp_pos >= count {
                tmp_byte_idx += 2;
                tmp_pos = 0;
            }
        }

        // Blit source row → output row with nearest-neighbor X scaling
        for ox in 0..width {
            let sx = (ox as u64 * src_w as u64 / width as u64) as usize;
            let rgb = if sx < src_w as usize { row_buf[sx] } else { 0 };
            put_pixel(fb, stride, ox, oy, to_fb_pixel(rgb, format));
        }

        // Advance master cursor if the next output row maps to a different source row
        let next_sy = if oy + 1 < height {
            ((oy + 1) as u64 * src_h as u64 / height as u64) as u32
        } else {
            src_h
        };
        if next_sy > sy {
            while src_pixel_idx < target_row_end && rle_byte_idx < rle_len {
                let (count, _idx) = rle_run(rle, rle_byte_idx);
                let remaining = count - rle_pos;
                let skip = target_row_end - src_pixel_idx;
                if skip >= remaining as usize {
                    src_pixel_idx += remaining as usize;
                    rle_byte_idx += 2;
                    rle_pos = 0;
                } else {
                    rle_pos += skip as u8;
                    src_pixel_idx += skip;
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEXT RENDERING (8×16 VGA font, directly to framebuffer)
// ═══════════════════════════════════════════════════════════════════════════

/// Draw a character at (x, y) pixel position.
unsafe fn draw_char(
    fb: *mut u32,
    stride: u32,
    format: u32,
    screen_w: u32,
    screen_h: u32,
    x: u32,
    y: u32,
    c: char,
    fg: u32,
    scale: u32,
) {
    let glyph = get_glyph_or_space(c);
    let fg_pixel = to_fb_pixel(fg, format);
    let char_w = (FONT_WIDTH as u32) * scale;
    let char_h = (FONT_HEIGHT as u32) * scale;

    for row in 0..FONT_HEIGHT as u32 {
        let bits = glyph[row as usize];
        for col in 0..FONT_WIDTH as u32 {
            if bits & (0x80 >> col) != 0 {
                // Draw scaled pixel block
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = x + col * scale + sx;
                        let py = y + row * scale + sy;
                        if px < screen_w && py < screen_h {
                            put_pixel(fb, stride, px, py, fg_pixel);
                        }
                    }
                }
            }
        }
    }
}

/// Draw a string at (x, y), returning the next Y position.
unsafe fn draw_string(
    fb: *mut u32,
    stride: u32,
    format: u32,
    screen_w: u32,
    screen_h: u32,
    x: u32,
    y: u32,
    text: &str,
    fg: u32,
    scale: u32,
) -> u32 {
    let char_w = (FONT_WIDTH as u32) * scale;
    let char_h = (FONT_HEIGHT as u32) * scale;
    let mut cx = x;
    let mut cy = y;

    for c in text.chars() {
        if c == '\n' {
            cx = x;
            cy += char_h + 2;
            continue;
        }
        if cx + char_w > screen_w {
            cx = x;
            cy += char_h + 2;
        }
        if cy + char_h > screen_h {
            break;
        }
        draw_char(fb, stride, format, screen_w, screen_h, cx, cy, c, fg, scale);
        cx += char_w;
    }
    cy + char_h + 2
}

// ═══════════════════════════════════════════════════════════════════════════
// SAD FACE DRAWING
// ═══════════════════════════════════════════════════════════════════════════

/// Draw a sad face emoticon centered at (cx, cy) with given radius.
unsafe fn draw_sad_face(
    fb: *mut u32,
    stride: u32,
    format: u32,
    screen_w: u32,
    screen_h: u32,
    cx: u32,
    cy: u32,
    radius: u32,
    color: u32,
) {
    let fg = to_fb_pixel(color, format);
    let r = radius as i64;
    let cxi = cx as i64;
    let cyi = cy as i64;

    // Draw circle outline (Bresenham-ish thick circle)
    for angle_step in 0..720 {
        // Use integer approximation of circle
        // x = cx + r*cos(theta), y = cy + r*sin(theta)
        // We'll use the midpoint circle algorithm instead
        let _ = angle_step;
    }

    // Midpoint circle - outline with thickness 3
    for thickness in 0..3i64 {
        let tr = r - thickness;
        if tr <= 0 {
            continue;
        }
        let mut x = 0i64;
        let mut y = tr;
        let mut d = 1 - tr;
        while x <= y {
            // Draw 8 octants
            for &(px, py) in &[
                (cxi + x, cyi + y),
                (cxi + y, cyi + x),
                (cxi - x, cyi + y),
                (cxi - y, cyi + x),
                (cxi + x, cyi - y),
                (cxi + y, cyi - x),
                (cxi - x, cyi - y),
                (cxi - y, cyi - x),
            ] {
                if px >= 0 && py >= 0 && (px as u32) < screen_w && (py as u32) < screen_h {
                    put_pixel(fb, stride, px as u32, py as u32, fg);
                }
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
    }

    // Left eye: filled circle at (-r/3, -r/4) with radius r/6
    let eye_r = (r / 6).max(3);
    let left_eye_x = cxi - r / 3;
    let left_eye_y = cyi - r / 4;
    let right_eye_x = cxi + r / 3;
    let right_eye_y = cyi - r / 4;

    for er in 0..=eye_r {
        let mut x = 0i64;
        let mut y = er;
        let mut d = 1 - er;
        while x <= y {
            for &(px, py) in &[
                (left_eye_x + x, left_eye_y + y),
                (left_eye_x + y, left_eye_y + x),
                (left_eye_x - x, left_eye_y + y),
                (left_eye_x - y, left_eye_y + x),
                (left_eye_x + x, left_eye_y - y),
                (left_eye_x + y, left_eye_y - x),
                (left_eye_x - x, left_eye_y - y),
                (left_eye_x - y, left_eye_y - x),
                (right_eye_x + x, right_eye_y + y),
                (right_eye_x + y, right_eye_y + x),
                (right_eye_x - x, right_eye_y + y),
                (right_eye_x - y, right_eye_y + x),
                (right_eye_x + x, right_eye_y - y),
                (right_eye_x + y, right_eye_y - x),
                (right_eye_x - x, right_eye_y - y),
                (right_eye_x - y, right_eye_y - x),
            ] {
                if px >= 0 && py >= 0 && (px as u32) < screen_w && (py as u32) < screen_h {
                    put_pixel(fb, stride, px as u32, py as u32, fg);
                }
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
    }

    // Sad mouth: arc (frown) - draw an upside-down arc below center
    let mouth_r = (r * 2 / 5).max(4);
    let mouth_cy = cyi + r / 3 + mouth_r; // center of frown arc (below the visible part)
    for thickness in 0..2i64 {
        let mr = mouth_r - thickness;
        if mr <= 0 {
            continue;
        }
        let mut x = 0i64;
        let mut y = mr;
        let mut d = 1 - mr;
        while x <= y {
            // Only draw the top half of the circle (the frown part)
            for &(px, py) in &[
                (cxi + x, mouth_cy - y),
                (cxi + y, mouth_cy - x),
                (cxi - x, mouth_cy - y),
                (cxi - y, mouth_cy - x),
            ] {
                // Only within the mouth region
                let in_range = py >= cyi + r / 4 && py <= cyi + r / 2 + r / 4;
                if in_range
                    && px >= 0
                    && py >= 0
                    && (px as u32) < screen_w
                    && (py as u32) < screen_h
                {
                    put_pixel(fb, stride, px as u32, py as u32, fg);
                }
            }
            x += 1;
            if d < 0 {
                d += 2 * x + 1;
            } else {
                y -= 1;
                d += 2 * (x - y) + 1;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HEX FORMATTING (no alloc)
// ═══════════════════════════════════════════════════════════════════════════

fn hex64(val: u64, buf: &mut [u8; 18]) -> &str {
    buf[0] = b'0';
    buf[1] = b'x';
    let digits = b"0123456789ABCDEF";
    for i in 0..16 {
        buf[2 + i] = digits[((val >> (60 - i * 4)) & 0xF) as usize];
    }
    // Safety: all bytes are ASCII
    unsafe { core::str::from_utf8_unchecked(&buf[..18]) }
}

fn hex32(val: u32, buf: &mut [u8; 10]) -> &str {
    buf[0] = b'0';
    buf[1] = b'x';
    let digits = b"0123456789ABCDEF";
    for i in 0..8 {
        buf[2 + i] = digits[((val >> (28 - i * 4)) & 0xF) as usize];
    }
    unsafe { core::str::from_utf8_unchecked(&buf[..10]) }
}

fn dec32(val: u32, buf: &mut [u8; 10]) -> &str {
    if val == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut n = val;
    let mut len = 0usize;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    buf[..len].reverse();
    unsafe { core::str::from_utf8_unchecked(&buf[..len]) }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC API — CRASH SCREEN
// ═══════════════════════════════════════════════════════════════════════════

/// Exception names for the crash screen (kept for show_panic_screen / future use).
#[allow(dead_code)]
const EXCEPTION_NAMES: [&str; 32] = [
    "DIVIDE_BY_ZERO",
    "DEBUG",
    "NMI",
    "BREAKPOINT",
    "OVERFLOW",
    "BOUND_RANGE_EXCEEDED",
    "INVALID_OPCODE",
    "DEVICE_NOT_AVAILABLE",
    "DOUBLE_FAULT",
    "COPROCESSOR_SEGMENT",
    "INVALID_TSS",
    "SEGMENT_NOT_PRESENT",
    "STACK_SEGMENT_FAULT",
    "GENERAL_PROTECTION_FAULT",
    "PAGE_FAULT",
    "RESERVED",
    "X87_FLOATING_POINT",
    "ALIGNMENT_CHECK",
    "MACHINE_CHECK",
    "SIMD_FLOATING_POINT",
    "VIRTUALIZATION",
    "CONTROL_PROTECTION",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
    "RESERVED",
];

/// Zero-alloc inline string builder for formatting crash screen lines.
struct Line {
    buf: [u8; 96],
    pos: usize,
}

impl Line {
    fn new() -> Self { Self { buf: [0; 96], pos: 0 } }
    fn clear(&mut self) { self.pos = 0; }
    fn push(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(96 - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&b[..n]);
        self.pos += n;
    }
    fn push_bytes(&mut self, b: &[u8]) {
        let n = b.len().min(96 - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&b[..n]);
        self.pos += n;
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.pos]).unwrap_or("")
    }
}

/// Display the full crash screen (BSoD) on the framebuffer.
///
/// Receives a rich [`morpheus_hwinit::CrashInfo`] containing all registers,
/// process context, human-readable explanation, and a kernel-mode backtrace.
///
/// # Safety
/// Must only be called when the system is in a fatal state.
/// Does not allocate — writes directly to the framebuffer.
pub unsafe fn show_crash_screen(info: &morpheus_hwinit::cpu::idt::CrashInfo) {
    let fb_info = match baremetal::get_framebuffer_info() {
        Some(fb) if fb.base != 0 && fb.width > 0 && fb.height > 0 => fb,
        _ => {
            puts("[BSOD] No framebuffer available\n");
            return;
        }
    };

    let fb = fb_info.base as *mut u32;
    let stride = fb_info.stride;
    let w = fb_info.width;
    let h = fb_info.height;
    let fmt = fb_info.format;

    puts("[BSOD] Drawing crash screen...\n");

    // 1. Background image (scaled Morpheus thumbnail)
    draw_background(fb, stride, w, h, fmt);

    // 2. Semi-transparent dark overlay panel
    let panel_w = w * 4 / 5;
    let panel_h = h * 5 / 6;
    let panel_x = (w - panel_w) / 2;
    let panel_y = (h - panel_h) / 2;

    for row in panel_y..panel_y + panel_h {
        for col in panel_x..panel_x + panel_w {
            let offset = (row as usize) * (stride as usize) + (col as usize);
            let existing = fb.add(offset).read_volatile();
            let er = (existing >> 16) & 0xFF;
            let eg = (existing >> 8) & 0xFF;
            let eb = existing & 0xFF;
            let blended = (er / 3) << 16 | (eg / 3) << 8 | eb / 3;
            fb.add(offset).write_volatile(to_fb_pixel(blended, fmt));
        }
    }

    // 3. Border
    let border_color = to_fb_pixel(0x661111, fmt);
    for t in 0..2u32 {
        for col in panel_x..panel_x + panel_w {
            put_pixel(fb, stride, col, panel_y + t, border_color);
            put_pixel(fb, stride, col, panel_y + panel_h - 1 - t, border_color);
        }
        for row in panel_y..panel_y + panel_h {
            put_pixel(fb, stride, panel_x + t, row, border_color);
            put_pixel(fb, stride, panel_x + panel_w - 1 - t, row, border_color);
        }
    }

    // Layout parameters
    let scale = if w >= 1920 { 2u32 } else { 1 };
    let margin = 24u32 * scale;
    let text_x = panel_x + margin;
    let mut text_y = panel_y + margin;

    let white = 0xFFFFFF;
    let red = 0xFF4444;
    let yellow = 0xFFCC44;
    let gray = 0x999999;
    let cyan = 0x66CCFF;

    let mut hbuf = [0u8; 18];
    let mut dbuf = [0u8; 10];
    let mut line = Line::new();

    // 4. Sad face + title
    let face_size = if w >= 1920 { 50u32 } else { 30 };
    let face_cx = text_x + face_size;
    let face_cy = text_y + face_size;
    draw_sad_face(fb, stride, fmt, w, h, face_cx, face_cy, face_size, white);

    let title_x = text_x + face_size * 2 + 30;
    let title_y = text_y + face_size / 2 - (FONT_HEIGHT as u32 * scale);
    text_y = draw_string(fb, stride, fmt, w, h, title_x, title_y, "MorpheusX", white, scale * 2);
    text_y = text_y.max(face_cy + face_size + 10);

    // 5. Subtitle
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y,
        "Your system ran into a problem and needs to stop.", gray, scale);
    text_y += 6 * scale;

    // 6. Process identification
    line.clear();
    line.push("Process: ");
    let name_end = info.process_name.iter().position(|&b| b == 0).unwrap_or(32);
    line.push_bytes(&info.process_name[..name_end]);
    line.push(" (PID ");
    line.push(dec32(info.pid, &mut dbuf));
    line.push(") ");
    line.push(if info.is_user_mode { "- User Mode (Ring 3)" } else { "- Kernel Mode (Ring 0)" });
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), cyan, scale);
    text_y += 6 * scale;

    // 7. Exception stop code
    line.clear();
    line.push("*** ");
    line.push(info.exception_name);
    line.push(" (0x");
    let vb = info.vector as u8;
    let hex_chars = b"0123456789ABCDEF";
    line.push_bytes(&[hex_chars[(vb >> 4) as usize], hex_chars[(vb & 0xF) as usize]]);
    line.push(") ***");
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), red, scale);

    // 8. Explanation
    if info.explanation_len > 0 {
        if let Ok(s) = core::str::from_utf8(&info.explanation[..info.explanation_len as usize]) {
            text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, s, yellow, scale);
        }
    }
    text_y += 6 * scale;

    // 9. Exception frame
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y,
        "--- Exception Frame ---", gray, scale);

    line.clear();
    line.push("RIP: "); line.push(hex64(info.rip, &mut hbuf));
    line.push("    RSP: "); line.push(hex64(info.rsp, &mut hbuf));
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);

    line.clear();
    line.push("CS:  "); line.push(hex64(info.cs, &mut hbuf));
    line.push("    SS:  "); line.push(hex64(info.ss, &mut hbuf));
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);

    line.clear();
    line.push("RFLAGS: "); line.push(hex64(info.rflags, &mut hbuf));
    line.push("  Error: "); line.push(hex64(info.error_code, &mut hbuf));
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);

    line.clear();
    line.push("CR2: "); line.push(hex64(info.cr2, &mut hbuf));
    line.push("    CR3: "); line.push(hex64(info.cr3, &mut hbuf));
    let cr_color = if info.vector == 14 { yellow } else { white };
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), cr_color, scale);

    // Page fault bit decode
    if info.vector == 14 {
        line.clear();
        line.push("Flags: ");
        if info.error_code & 1 != 0 { line.push("PRESENT "); } else { line.push("NOT_PRESENT "); }
        if info.error_code & 2 != 0 { line.push("WRITE "); } else { line.push("READ "); }
        if info.error_code & 4 != 0 { line.push("USER "); } else { line.push("SUPERVISOR "); }
        if info.error_code & 8 != 0 { line.push("RSVD "); }
        if info.error_code & 16 != 0 { line.push("IFETCH "); }
        text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), yellow, scale);
    }
    text_y += 4 * scale;

    // 10. All general-purpose registers (2 per line)
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y,
        "--- Registers ---", gray, scale);

    let reg_pairs: [(&str, u64, &str, u64); 7] = [
        ("RAX: ", info.rax, "RBX: ", info.rbx),
        ("RCX: ", info.rcx, "RDX: ", info.rdx),
        ("RSI: ", info.rsi, "RDI: ", info.rdi),
        ("RBP: ", info.rbp, "R8:  ", info.r8),
        ("R9:  ", info.r9,  "R10: ", info.r10),
        ("R11: ", info.r11, "R12: ", info.r12),
        ("R13: ", info.r13, "R14: ", info.r14),
    ];
    for &(na, va, nb, vb) in reg_pairs.iter() {
        line.clear();
        line.push(na); line.push(hex64(va, &mut hbuf));
        line.push("    ");
        line.push(nb); line.push(hex64(vb, &mut hbuf));
        text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);
    }
    line.clear();
    line.push("R15: "); line.push(hex64(info.r15, &mut hbuf));
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);
    text_y += 4 * scale;

    // 11. Backtrace (kernel-mode only, if available)
    if info.backtrace_depth > 0 {
        text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y,
            "--- Backtrace ---", gray, scale);
        let show = (info.backtrace_depth as usize).min(10); // cap to avoid overflow
        for i in 0..show {
            line.clear();
            line.push("#");
            line.push(dec32(i as u32, &mut dbuf));
            line.push("  ");
            line.push(hex64(info.backtrace[i], &mut hbuf));
            text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, line.as_str(), white, scale);
        }
        text_y += 4 * scale;
    }

    // 12. Footer
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y,
        "System halted. Power off or reset to restart.", gray, scale);
    let _ = text_y;

    puts("[BSOD] Crash screen rendered\n");
}

/// Display a panic screen with source location.
///
/// # Safety
/// Same constraints as `show_crash_screen`.
pub unsafe fn show_panic_screen(file: &str, line: u32, col: u32) {
    let fb_info = match baremetal::get_framebuffer_info() {
        Some(fb) if fb.base != 0 && fb.width > 0 && fb.height > 0 => fb,
        _ => return,
    };

    let fb = fb_info.base as *mut u32;
    let stride = fb_info.stride;
    let w = fb_info.width;
    let h = fb_info.height;
    let fmt = fb_info.format;

    // Draw background
    draw_background(fb, stride, w, h, fmt);

    // Dark overlay
    let panel_w = w * 3 / 4;
    let panel_h = h * 3 / 4;
    let panel_x = (w - panel_w) / 2;
    let panel_y = (h - panel_h) / 2;

    for row in panel_y..panel_y + panel_h {
        for col in panel_x..panel_x + panel_w {
            let offset = (row as usize) * (stride as usize) + (col as usize);
            let existing = fb.add(offset).read_volatile();
            let er = (existing >> 16) & 0xFF;
            let eg = (existing >> 8) & 0xFF;
            let eb = existing & 0xFF;
            let blended = (er / 3) << 16 | (eg / 3) << 8 | eb / 3;
            fb.add(offset).write_volatile(to_fb_pixel(blended, fmt));
        }
    }

    // Border
    let border_color = to_fb_pixel(0x661111, fmt);
    for t in 0..2u32 {
        for c in panel_x..panel_x + panel_w {
            put_pixel(fb, stride, c, panel_y + t, border_color);
            put_pixel(fb, stride, c, panel_y + panel_h - 1 - t, border_color);
        }
        for r in panel_y..panel_y + panel_h {
            put_pixel(fb, stride, panel_x + t, r, border_color);
            put_pixel(fb, stride, panel_x + panel_w - 1 - t, r, border_color);
        }
    }

    let scale = if w >= 1920 { 2u32 } else { 1 };
    let margin = 30u32 * scale;
    let text_x = panel_x + margin;
    let mut text_y = panel_y + margin;

    let white = 0xFFFFFF;
    let red = 0xFF4444;
    let gray = 0x999999;

    // Sad face
    let face_size = if w >= 1920 { 50u32 } else { 30 };
    draw_sad_face(fb, stride, fmt, w, h, text_x + face_size, text_y + face_size, face_size, white);

    // Title
    let title_x = text_x + face_size * 2 + 30;
    text_y = draw_string(
        fb, stride, fmt, w, h,
        title_x, text_y + face_size / 2,
        "MorpheusX",
        white,
        scale * 2,
    );
    text_y = text_y.max(panel_y + margin + face_size * 2 + 20);

    text_y = draw_string(
        fb, stride, fmt, w, h,
        text_x, text_y,
        "Your system ran into a problem and needs to stop.",
        gray,
        scale,
    );
    text_y += 10 * scale;

    text_y = draw_string(
        fb, stride, fmt, w, h,
        text_x, text_y,
        "*** KERNEL PANIC ***",
        red,
        scale,
    );
    text_y += 8 * scale;

    // File location
    text_y = draw_string(fb, stride, fmt, w, h, text_x, text_y, "Location:", gray, scale);

    // File name (may be long, just print as-is)
    text_y = draw_string(fb, stride, fmt, w, h, text_x + 20, text_y, file, white, scale);

    // Line number
    let mut buf32 = [0u8; 10];
    let mut line_info = [0u8; 20];
    line_info[..6].copy_from_slice(b"Line: ");
    let d = dec32(line, &mut buf32);
    let dlen = d.len();
    line_info[6..6 + dlen].copy_from_slice(d.as_bytes());
    if let Ok(s) = core::str::from_utf8(&line_info[..6 + dlen]) {
        text_y = draw_string(fb, stride, fmt, w, h, text_x + 20, text_y, s, white, scale);
    }

    text_y += 16 * scale;

    text_y = draw_string(
        fb, stride, fmt, w, h,
        text_x, text_y,
        "System halted. Power off or reset to restart.",
        gray,
        scale,
    );
}
