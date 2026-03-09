pub mod launcher;
pub mod panel;
pub mod wallpaper;

pub const PANEL_H: u32 = 30;
pub const START_BTN_W: u32 = 84;
pub const LAUNCHER_W: u32 = 300;
pub const LAUNCHER_H: u32 = 220;
pub const ICON_SIZE: u32 = 56;

pub const DESKTOP_RGB: (u8, u8, u8) = (26, 26, 46);
pub const PANEL_BG_RGB: (u8, u8, u8) = (18, 20, 30);
pub const START_RGB: (u8, u8, u8) = (0, 85, 0);
pub const START_ACTIVE_RGB: (u8, u8, u8) = (0, 110, 42);
pub const LAUNCHER_BG_RGB: (u8, u8, u8) = (28, 30, 42);
pub const ICON_BG_RGB: (u8, u8, u8) = (40, 60, 90);
pub const ICON_INNER_RGB: (u8, u8, u8) = (18, 24, 40);

pub struct ShellState {
    // surface buffer — private offscreen FB given by kernel because compd owns the real one
    pub surface_ptr: *mut u32,
    pub fb_w: u32,
    pub fb_h: u32,
    pub fb_stride: u32, // stride in PIXELS (fb_info.stride / 4). yes confusing. no we can't change it.
    pub is_bgrx: bool,

    // wallpaper island
    pub wallpaper_dirty: bool,

    // panel island
    pub panel_dirty: bool,

    // launcher island
    pub launcher_open: bool,
    pub launcher_dirty: bool,

    // input state — shelld reads forwarded mouse from compd
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub last_buttons: u8,
}

impl ShellState {
    pub fn new(
        surface_ptr: *mut u32,
        fb_w: u32,
        fb_h: u32,
        fb_stride_px: u32,
        is_bgrx: bool,
    ) -> Self {
        Self {
            surface_ptr,
            fb_w,
            fb_h,
            fb_stride: fb_stride_px,
            is_bgrx,
            wallpaper_dirty: true,
            panel_dirty: true,
            launcher_open: false,
            launcher_dirty: false,
            // init at center. same as compd. so when compd forwards deltas, our position tracks.
            mouse_x: (fb_w / 2) as i32,
            mouse_y: (fb_h / 2) as i32,
            last_buttons: 0,
        }
    }

    // pack the pixel. bgrx not rgbx. the b comes first because uefi said so and uefi answers to no one.
    #[inline(always)]
    pub fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
    }
}

// --- raw pixel primitives for writing into shelld's own surface buffer ---

#[inline(always)]
pub fn raw_fill(buf: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, px: u32) {
    for row in y..y + h {
        let off = (row * stride + x) as usize;
        unsafe {
            let ptr = buf.add(off);
            for col in 0..w as usize {
                ptr.add(col).write(px);
            }
        }
    }
}

#[inline(always)]
pub fn raw_glyph(
    buf: *mut u32,
    stride: u32,
    gx: u32,
    gy: u32,
    glyph: &[u8; 16],
    fg: u32,
    bg: u32,
    fb_h: u32,
) {
    for row in 0u32..16 {
        let py = gy + row;
        if py >= fb_h {
            break;
        }
        let bits = glyph[row as usize];
        let base = (py * stride + gx) as usize;
        for col in 0u32..8 {
            let is_fg = (bits >> (7 - col)) & 1 == 1;
            unsafe {
                buf.add(base + col as usize)
                    .write(if is_fg { fg } else { bg });
            }
        }
    }
}

pub fn draw_text(
    state: &ShellState,
    x: u32,
    y: u32,
    text: &str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
) {
    let fg_px = state.pack(fg.0, fg.1, fg.2);
    let bg_px = state.pack(bg.0, bg.1, bg.2);
    let font_w = 8u32;

    for (ci, ch) in text.chars().enumerate() {
        let gx = x + ci as u32 * font_w;
        if gx + font_w > state.fb_w {
            break;
        }
        let glyph = crate::font::get_glyph_or_space(ch);
        raw_glyph(
            state.surface_ptr,
            state.fb_stride,
            gx,
            y,
            glyph,
            fg_px,
            bg_px,
            state.fb_h,
        );
    }
}
