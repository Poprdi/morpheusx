//! Desktop environment appearance profile shared across settings, shelld, and compd.

use crate::persist;

pub const APPEARANCE_KEY: &str = "de_appearance_v1";
pub const APPEARANCE_KEY_LEGACY: &str = "de.appearance.v1";
const MAGIC: [u8; 4] = *b"MDE1";
const VERSION: u8 = 1;
const ENCODED_LEN: usize = 26;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DesktopAppearance {
    pub dark_mode: bool,
    pub accent_rgb: (u8, u8, u8),
    pub desktop_rgb: (u8, u8, u8),
    pub panel_rgb: (u8, u8, u8),
    pub start_rgb: (u8, u8, u8),
    pub title_focus_rgb: (u8, u8, u8),
    pub border_focus_rgb: (u8, u8, u8),
}

impl DesktopAppearance {
    pub const fn default_dark() -> Self {
        Self {
            dark_mode: true,
            accent_rgb: (0, 230, 118),
            desktop_rgb: (26, 26, 46),
            panel_rgb: (18, 20, 30),
            start_rgb: (0, 85, 0),
            title_focus_rgb: (0, 85, 0),
            border_focus_rgb: (0, 170, 0),
        }
    }

    pub const fn default_light() -> Self {
        Self {
            dark_mode: false,
            accent_rgb: (0, 132, 96),
            desktop_rgb: (236, 238, 245),
            panel_rgb: (220, 223, 230),
            start_rgb: (0, 132, 96),
            title_focus_rgb: (0, 132, 96),
            border_focus_rgb: (0, 165, 120),
        }
    }

    pub fn from_theme_choice(dark_mode: bool, accent_rgb: (u8, u8, u8)) -> Self {
        let (ar, ag, ab) = accent_rgb;
        let title = scale_rgb(accent_rgb, if dark_mode { 38 } else { 68 });
        let border = scale_rgb(accent_rgb, if dark_mode { 76 } else { 92 });
        let start = scale_rgb(accent_rgb, if dark_mode { 34 } else { 74 });
        let desktop = if dark_mode {
            (26, 26, 46)
        } else {
            (236, 238, 245)
        };
        let panel = if dark_mode {
            (18, 20, 30)
        } else {
            (220, 223, 230)
        };

        Self {
            dark_mode,
            accent_rgb: (ar, ag, ab),
            desktop_rgb: desktop,
            panel_rgb: panel,
            start_rgb: start,
            title_focus_rgb: title,
            border_focus_rgb: border,
        }
    }

    pub fn load() -> Option<Self> {
        let mut buf = [0u8; ENCODED_LEN];
        if let Ok(n) = persist::get(APPEARANCE_KEY, &mut buf) {
            if n == ENCODED_LEN {
                if let Some(v) = decode(&buf) {
                    return Some(v);
                }
            }
        }

        if let Ok(n) = persist::get(APPEARANCE_KEY_LEGACY, &mut buf) {
            if n == ENCODED_LEN {
                return decode(&buf);
            }
        }

        None
    }

    pub fn store(&self) -> Result<(), u64> {
        let buf = encode(self);
        match persist::put(APPEARANCE_KEY, &buf) {
            Ok(()) => Ok(()),
            Err(_) => persist::put(APPEARANCE_KEY_LEGACY, &buf),
        }
    }
}

#[inline(always)]
fn scale8(v: u8, pct: u16) -> u8 {
    let x = (v as u16).saturating_mul(pct) / 100;
    x.min(255) as u8
}

#[inline(always)]
fn scale_rgb(rgb: (u8, u8, u8), pct: u16) -> (u8, u8, u8) {
    (scale8(rgb.0, pct), scale8(rgb.1, pct), scale8(rgb.2, pct))
}

fn encode(a: &DesktopAppearance) -> [u8; ENCODED_LEN] {
    let mut out = [0u8; ENCODED_LEN];
    out[0..4].copy_from_slice(&MAGIC);
    out[4] = VERSION;
    out[5] = if a.dark_mode { 1 } else { 0 };

    write_rgb(&mut out, 8, a.accent_rgb);
    write_rgb(&mut out, 11, a.desktop_rgb);
    write_rgb(&mut out, 14, a.panel_rgb);
    write_rgb(&mut out, 17, a.start_rgb);
    write_rgb(&mut out, 20, a.title_focus_rgb);
    write_rgb(&mut out, 23, a.border_focus_rgb);
    out
}

fn decode(buf: &[u8; ENCODED_LEN]) -> Option<DesktopAppearance> {
    if buf[0..4] != MAGIC || buf[4] != VERSION {
        return None;
    }

    Some(DesktopAppearance {
        dark_mode: buf[5] != 0,
        accent_rgb: read_rgb(buf, 8),
        desktop_rgb: read_rgb(buf, 11),
        panel_rgb: read_rgb(buf, 14),
        start_rgb: read_rgb(buf, 17),
        title_focus_rgb: read_rgb(buf, 20),
        border_focus_rgb: read_rgb(buf, 23),
    })
}

#[inline(always)]
fn write_rgb(buf: &mut [u8; ENCODED_LEN], i: usize, rgb: (u8, u8, u8)) {
    buf[i] = rgb.0;
    buf[i + 1] = rgb.1;
    buf[i + 2] = rgb.2;
}

#[inline(always)]
fn read_rgb(buf: &[u8; ENCODED_LEN], i: usize) -> (u8, u8, u8) {
    (buf[i], buf[i + 1], buf[i + 2])
}
