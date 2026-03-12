// direct framebuffer text and shape primitives.
// no canvas abstraction. no trait dispatch. just pointer math and pixel writes.
// this is a standalone process with its own mapped surface.



// vga 8x16 font constants
pub const FONT_W: u32 = 8;
pub const FONT_H: u32 = 16;

// embedded vga font — 95 printable ascii glyphs (0x20..=0x7E)
pub static FONT_DATA: [[u8; 16]; 95] = include!("font_data.inc");

#[inline(always)]
pub fn put_pixel(surface: *mut u32, stride: u32, x: u32, y: u32, color: u32) {
    unsafe {
        let offset = (y * stride + x) as isize;
        surface.offset(offset).write_volatile(color);
    }
}

pub fn fill_rect(surface: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32, fb_w: u32, fb_h: u32) {
    // clamp to bounds
    let x1 = x.min(fb_w);
    let y1 = y.min(fb_h);
    let x2 = (x + w).min(fb_w);
    let y2 = (y + h).min(fb_h);
    if x1 >= x2 || y1 >= y2 {
        return;
    }
    let row_len = (x2 - x1) as usize;
    for row in y1..y2 {
        unsafe {
            let start = surface.offset((row * stride + x1) as isize);
            for col in 0..row_len {
                start.add(col).write_volatile(color);
            }
        }
    }
}

pub fn draw_char(surface: *mut u32, stride: u32, x: u32, y: u32, ch: u8, fg: u32, bg: u32, fb_w: u32, fb_h: u32) {
    let idx = if ch >= 0x20 && ch <= 0x7E {
        (ch - 0x20) as usize
    } else {
        0 // space fallback
    };
    let glyph = &FONT_DATA[idx];

    for row in 0..16u32 {
        let py = y + row;
        if py >= fb_h {
            break;
        }
        let bits = glyph[row as usize];
        for col in 0..8u32 {
            let px = x + col;
            if px >= fb_w {
                break;
            }
            let color = if (bits >> (7 - col)) & 1 == 1 { fg } else { bg };
            unsafe {
                surface.offset((py * stride + px) as isize).write_volatile(color);
            }
        }
    }
}

pub fn draw_str(surface: *mut u32, stride: u32, x: u32, y: u32, s: &str, fg: u32, bg: u32, fb_w: u32, fb_h: u32) {
    let mut cx = x;
    for &b in s.as_bytes() {
        if cx + FONT_W > fb_w {
            break;
        }
        draw_char(surface, stride, cx, y, b, fg, bg, fb_w, fb_h);
        cx += FONT_W;
    }
}

// draw string with max width (truncates with ".." if too long)
pub fn draw_str_trunc(surface: *mut u32, stride: u32, x: u32, y: u32, s: &str, fg: u32, bg: u32, fb_w: u32, fb_h: u32, max_chars: usize) {
    if max_chars == 0 {
        return;
    }
    let total_chars = s.chars().count();
    if total_chars <= max_chars {
        draw_str(surface, stride, x, y, s, fg, bg, fb_w, fb_h);
    } else {
        if max_chars <= 2 {
            draw_str(surface, stride, x, y, "..", fg, bg, fb_w, fb_h);
            return;
        }

        let keep = max_chars - 2;
        let mut end = 0usize;
        for (i, (bi, _)) in s.char_indices().enumerate() {
            if i == keep {
                break;
            }
            end = bi;
        }
        // include last kept character boundary
        if keep > 0 {
            if let Some((bi, ch)) = s.char_indices().nth(keep - 1) {
                end = bi + ch.len_utf8();
            }
        }

        let trunc = &s[..end.min(s.len())];
        draw_str(surface, stride, x, y, trunc, fg, bg, fb_w, fb_h);
        let cx = x + (keep as u32) * FONT_W;
        draw_str(surface, stride, cx, y, "..", fg, bg, fb_w, fb_h);
    }
}

// horizontal line
pub fn hline(surface: *mut u32, stride: u32, x: u32, y: u32, w: u32, color: u32, fb_w: u32, fb_h: u32) {
    fill_rect(surface, stride, x, y, w, 1, color, fb_w, fb_h);
}

// vertical line
pub fn vline(surface: *mut u32, stride: u32, x: u32, y: u32, h: u32, color: u32, fb_w: u32, fb_h: u32) {
    fill_rect(surface, stride, x, y, 1, h, color, fb_w, fb_h);
}

// outlined rectangle
pub fn rect_outline(surface: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, color: u32, fb_w: u32, fb_h: u32) {
    hline(surface, stride, x, y, w, color, fb_w, fb_h);
    hline(surface, stride, x, y + h.saturating_sub(1), w, color, fb_w, fb_h);
    vline(surface, stride, x, y, h, color, fb_w, fb_h);
    vline(surface, stride, x + w.saturating_sub(1), y, h, color, fb_w, fb_h);
}

// integer to decimal string — no alloc, no format!, no core::fmt.
// returns the number of bytes written to `buf`.
pub fn u64_to_str(val: u64, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
        }
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    let len = n.min(buf.len());
    for i in 0..len {
        buf[i] = tmp[n - 1 - i];
    }
    len
}

// format bytes as human readable (KB/MB/GB)
pub fn format_bytes(bytes: u64, buf: &mut [u8]) -> usize {
    if bytes < 1024 {
        let n = u64_to_str(bytes, buf);
        let suffix = b" B";
        let end = (n + suffix.len()).min(buf.len());
        buf[n..end].copy_from_slice(&suffix[..end - n]);
        return end;
    }
    let (val, suffix) = if bytes < 1024 * 1024 {
        (bytes / 1024, &b" KB"[..])
    } else if bytes < 1024 * 1024 * 1024 {
        (bytes / (1024 * 1024), &b" MB"[..])
    } else {
        (bytes / (1024 * 1024 * 1024), &b" GB"[..])
    };
    let n = u64_to_str(val, buf);
    let end = (n + suffix.len()).min(buf.len());
    buf[n..end].copy_from_slice(&suffix[..end - n]);
    end
}

// format ip address (network byte order u32 -> dotted decimal)
pub fn format_ip(ip: u32, buf: &mut [u8]) -> usize {
    let octets = ip.to_be_bytes();
    let mut pos = 0;
    for (i, &o) in octets.iter().enumerate() {
        let n = u64_to_str(o as u64, &mut buf[pos..]);
        pos += n;
        if i < 3 && pos < buf.len() {
            buf[pos] = b'.';
            pos += 1;
        }
    }
    pos
}

// format mac address
pub fn format_mac(mac: &[u8; 6], buf: &mut [u8]) -> usize {
    let hex = b"0123456789ABCDEF";
    let mut pos = 0;
    for (i, &b) in mac.iter().enumerate() {
        if pos + 2 > buf.len() { break; }
        buf[pos] = hex[(b >> 4) as usize];
        buf[pos + 1] = hex[(b & 0x0F) as usize];
        pos += 2;
        if i < 5 && pos < buf.len() {
            buf[pos] = b':';
            pos += 1;
        }
    }
    pos
}

// format uptime from ticks
pub fn format_uptime(ms: u64, buf: &mut [u8]) -> usize {
    let secs = ms / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;

    let mut pos = 0;
    if days > 0 {
        let n = u64_to_str(days, &mut buf[pos..]);
        pos += n;
        let s = b"d ";
        let end = (pos + s.len()).min(buf.len());
        buf[pos..end].copy_from_slice(&s[..end - pos]);
        pos = end;
    }
    let n = u64_to_str(hours % 24, &mut buf[pos..]);
    pos += n;
    if pos < buf.len() { buf[pos] = b'h'; pos += 1; }
    if pos < buf.len() { buf[pos] = b' '; pos += 1; }
    let n = u64_to_str(mins % 60, &mut buf[pos..]);
    pos += n;
    if pos < buf.len() { buf[pos] = b'm'; pos += 1; }
    if pos < buf.len() { buf[pos] = b' '; pos += 1; }
    let n = u64_to_str(secs % 60, &mut buf[pos..]);
    pos += n;
    if pos < buf.len() { buf[pos] = b's'; pos += 1; }
    pos
}

// percentage bar — a horizontal bar filled proportionally
pub fn draw_bar(surface: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32,
                fraction: u32, max: u32, fg: u32, bg: u32, border: u32, fb_w: u32, fb_h: u32) {
    rect_outline(surface, stride, x, y, w, h, border, fb_w, fb_h);
    let inner_w = w.saturating_sub(2);
    let inner_h = h.saturating_sub(2);
    fill_rect(surface, stride, x + 1, y + 1, inner_w, inner_h, bg, fb_w, fb_h);
    if max > 0 {
        let fill = (inner_w as u64 * fraction as u64 / max as u64) as u32;
        if fill > 0 {
            fill_rect(surface, stride, x + 1, y + 1, fill, inner_h, fg, fb_w, fb_h);
        }
    }
}
