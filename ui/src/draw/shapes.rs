use crate::canvas::Canvas;
use crate::color::Color;
use crate::rect::Rect;

pub fn hline(canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, color: Color) {
    canvas.fill_rect(x, y, w, 1, color);
}

pub fn vline(canvas: &mut dyn Canvas, x: u32, y: u32, h: u32, color: Color) {
    canvas.fill_rect(x, y, 1, h, color);
}

pub fn rect_fill(canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, h: u32, color: Color) {
    canvas.fill_rect(x, y, w, h, color);
}

pub fn rect_outline(
    canvas: &mut dyn Canvas,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    thickness: u32,
    color: Color,
) {
    if w == 0 || h == 0 || thickness == 0 {
        return;
    }
    let t = thickness.min(w / 2).min(h / 2).max(1);
    // Top edge
    canvas.fill_rect(x, y, w, t, color);
    // Bottom edge
    canvas.fill_rect(x, y.saturating_add(h).saturating_sub(t), w, t, color);
    // Left edge (between top and bottom)
    if h > t * 2 {
        canvas.fill_rect(x, y + t, t, h - t * 2, color);
        // Right edge
        canvas.fill_rect(x.saturating_add(w).saturating_sub(t), y + t, t, h - t * 2, color);
    }
}

pub fn circle_outline(canvas: &mut dyn Canvas, cx: u32, cy: u32, r: u32, color: Color) {
    if r == 0 {
        canvas.put_pixel(cx, cy, color);
        return;
    }

    let mut x = 0i32;
    let mut y = r as i32;
    let mut d = 1 - r as i32;

    let plot = |canvas: &mut dyn Canvas, ox: i32, oy: i32| {
        if ox >= 0 && oy >= 0 {
            canvas.put_pixel(ox as u32, oy as u32, color);
        }
    };

    while x <= y {
        let cxi = cx as i32;
        let cyi = cy as i32;
        plot(canvas, cxi + x, cyi + y);
        plot(canvas, cxi - x, cyi + y);
        plot(canvas, cxi + x, cyi - y);
        plot(canvas, cxi - x, cyi - y);
        plot(canvas, cxi + y, cyi + x);
        plot(canvas, cxi - y, cyi + x);
        plot(canvas, cxi + y, cyi - x);
        plot(canvas, cxi - y, cyi - x);

        if d < 0 {
            d += 2 * x + 3;
        } else {
            d += 2 * (x - y) + 5;
            y -= 1;
        }
        x += 1;
    }
}

pub fn circle_fill(canvas: &mut dyn Canvas, cx: u32, cy: u32, r: u32, color: Color) {
    if r == 0 {
        canvas.put_pixel(cx, cy, color);
        return;
    }

    let mut x = 0i32;
    let mut y = r as i32;
    let mut d = 1 - r as i32;

    let hfill = |canvas: &mut dyn Canvas, from_x: i32, to_x: i32, py: i32| {
        if py < 0 || from_x > to_x {
            return;
        }
        let fx = from_x.max(0) as u32;
        let w = (to_x - from_x + 1).max(0) as u32;
        canvas.fill_rect(fx, py as u32, w, 1, color);
    };

    while x <= y {
        let cxi = cx as i32;
        let cyi = cy as i32;
        hfill(canvas, cxi - x, cxi + x, cyi + y);
        hfill(canvas, cxi - x, cxi + x, cyi - y);
        hfill(canvas, cxi - y, cxi + y, cyi + x);
        hfill(canvas, cxi - y, cxi + y, cyi - x);

        if d < 0 {
            d += 2 * x + 3;
        } else {
            d += 2 * (x - y) + 5;
            y -= 1;
        }
        x += 1;
    }
}

pub fn rounded_rect_fill(
    canvas: &mut dyn Canvas,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: Color,
) {
    if w == 0 || h == 0 {
        return;
    }

    let clip = match Rect::new(x, y, w, h).intersect(canvas.bounds()) {
        Some(c) => c,
        None => return,
    };

    let r = radius.min(w / 2).min(h / 2);
    if r == 0 {
        canvas.fill_rect(clip.x, clip.y, clip.w, clip.h, color);
        return;
    }

    // Straight middle section (between corner rows)
    if h > r * 2 {
        canvas.fill_rect(x, y + r, w, h - r * 2, color);
    }

    // Corner arcs via midpoint circle scanlines
    let mut cx_i = 0i32;
    let mut cy_i = r as i32;
    let mut d = 1 - r as i32;

    while cx_i <= cy_i {
        // Top corners
        let top_y1 = y + r - cy_i as u32;
        let top_y2 = y + r - cx_i as u32;
        let bot_y1 = y + h - 1 - r + cy_i as u32;
        let bot_y2 = y + h - 1 - r + cx_i as u32;

        let x_span1 = cx_i as u32;
        let x_span2 = cy_i as u32;

        // Horizontal fills for the rounded sections
        canvas.fill_rect(x + r - x_span1, top_y1, w - 2 * (r - x_span1), 1, color);
        canvas.fill_rect(x + r - x_span2, top_y2, w - 2 * (r - x_span2), 1, color);
        canvas.fill_rect(x + r - x_span1, bot_y1, w - 2 * (r - x_span1), 1, color);
        canvas.fill_rect(x + r - x_span2, bot_y2, w - 2 * (r - x_span2), 1, color);

        if d < 0 {
            d += 2 * cx_i + 3;
        } else {
            d += 2 * (cx_i - cy_i) + 5;
            cy_i -= 1;
        }
        cx_i += 1;
    }
}

pub fn rounded_rect_outline(
    canvas: &mut dyn Canvas,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: u32,
    color: Color,
) {
    if w == 0 || h == 0 {
        return;
    }

    let r = radius.min(w / 2).min(h / 2);
    if r == 0 {
        rect_outline(canvas, x, y, w, h, 1, color);
        return;
    }

    // Straight edges
    if w > r * 2 {
        hline(canvas, x + r, y, w - r * 2, color);
        hline(canvas, x + r, y + h - 1, w - r * 2, color);
    }
    if h > r * 2 {
        vline(canvas, x, y + r, h - r * 2, color);
        vline(canvas, x + w - 1, y + r, h - r * 2, color);
    }

    // Corner arcs
    let mut cx_i = 0i32;
    let mut cy_i = r as i32;
    let mut d = 1 - r as i32;

    let plot_corners = |canvas: &mut dyn Canvas, ox: i32, oy: i32| {
        let tl_x = (x + r) as i32 - ox;
        let tl_y = (y + r) as i32 - oy;
        let tr_x = (x + w - 1 - r) as i32 + ox;
        let tr_y = (y + r) as i32 - oy;
        let bl_x = (x + r) as i32 - ox;
        let bl_y = (y + h - 1 - r) as i32 + oy;
        let br_x = (x + w - 1 - r) as i32 + ox;
        let br_y = (y + h - 1 - r) as i32 + oy;

        if tl_x >= 0 && tl_y >= 0 {
            canvas.put_pixel(tl_x as u32, tl_y as u32, color);
        }
        if tr_x >= 0 && tr_y >= 0 {
            canvas.put_pixel(tr_x as u32, tr_y as u32, color);
        }
        if bl_x >= 0 && bl_y >= 0 {
            canvas.put_pixel(bl_x as u32, bl_y as u32, color);
        }
        if br_x >= 0 && br_y >= 0 {
            canvas.put_pixel(br_x as u32, br_y as u32, color);
        }
    };

    while cx_i <= cy_i {
        plot_corners(canvas, cx_i, cy_i);
        plot_corners(canvas, cy_i, cx_i);

        if d < 0 {
            d += 2 * cx_i + 3;
        } else {
            d += 2 * (cx_i - cy_i) + 5;
            cy_i -= 1;
        }
        cx_i += 1;
    }
}

pub fn line(canvas: &mut dyn Canvas, x0: u32, y0: u32, x1: u32, y1: u32, color: Color) {
    let mut x0 = x0 as i32;
    let mut y0 = y0 as i32;
    let x1 = x1 as i32;
    let y1 = y1 as i32;

    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && y0 >= 0 {
            canvas.put_pixel(x0 as u32, y0 as u32, color);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
