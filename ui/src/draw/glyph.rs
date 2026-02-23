use crate::canvas::Canvas;
use crate::color::Color;

pub fn draw_glyph(
    canvas: &mut dyn Canvas,
    gx: u32,
    gy: u32,
    glyph: &[u8; 16],
    fg: Color,
    bg: Color,
) {
    let cw = canvas.width();
    let ch = canvas.height();

    for (row_idx, &row_bits) in glyph.iter().enumerate() {
        let py = gy + row_idx as u32;
        if py >= ch {
            break;
        }

        // Fast path: entire row is one color
        if row_bits == 0x00 {
            if gx + 8 <= cw {
                canvas.fill_rect(gx, py, 8, 1, bg);
            }
            continue;
        }
        if row_bits == 0xFF {
            if gx + 8 <= cw {
                canvas.fill_rect(gx, py, 8, 1, fg);
            }
            continue;
        }

        // Run-length encode: collapse consecutive same-color pixels
        let mut x = 0u32;
        while x < 8 {
            if gx + x >= cw {
                break;
            }
            let is_fg = (row_bits >> (7 - x)) & 1 == 1;
            let color = if is_fg { fg } else { bg };
            let run_start = x;
            x += 1;
            while x < 8 && ((row_bits >> (7 - x)) & 1 == 1) == is_fg {
                x += 1;
            }
            let run_len = x - run_start;
            let clipped_len = run_len.min(cw.saturating_sub(gx + run_start));
            if clipped_len > 0 {
                canvas.fill_rect(gx + run_start, py, clipped_len, 1, color);
            }
        }
    }
}

pub fn draw_char(
    canvas: &mut dyn Canvas,
    x: u32,
    y: u32,
    c: char,
    fg: Color,
    bg: Color,
    font_data: &[[u8; 16]],
) {
    let idx = c as usize;
    let glyph = if (0x20..=0x7E).contains(&idx) {
        &font_data[idx - 0x20]
    } else {
        &font_data[0] // space fallback
    };
    draw_glyph(canvas, x, y, glyph, fg, bg);
}

pub fn draw_string(
    canvas: &mut dyn Canvas,
    x: u32,
    y: u32,
    s: &str,
    fg: Color,
    bg: Color,
    font_data: &[[u8; 16]],
) {
    let mut cx = x;
    for c in s.chars() {
        if cx + 8 > canvas.width() {
            break;
        }
        draw_char(canvas, cx, y, c, fg, bg, font_data);
        cx += 8;
    }
}
