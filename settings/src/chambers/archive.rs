// archive of echoes — changelog timeline viewer.
// every change ever applied in this session, searchable, with destructive markers.
// the system's memory. read-only — you cannot un-echo what has been spoken.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::SettingsApp;
use crate::widgets;

const VISIBLE_ENTRIES: usize = 16;

pub struct ArchiveChamber {
    pub scroll_offset: usize,
    pub selected: usize,
    pub search_buf: [u8; 32],
    pub search_len: usize,
    pub searching: bool,
}

impl ArchiveChamber {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            selected: 0,
            search_buf: [0; 32],
            search_len: 0,
            searching: false,
        }
    }

    pub fn widget_count(&self) -> usize {
        3 + VISIBLE_ENTRIES
    }

    pub fn activate(&mut self, idx: usize) {
        match idx {
            0 => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            1 => {
                // scroll down — can't check max without changelog len, just increment
                self.scroll_offset += 1;
            }
            2 => {
                self.searching = !self.searching;
                if !self.searching {
                    self.search_len = 0;
                }
            }
            _ => {
                self.selected = idx - 3 + self.scroll_offset;
            }
        }
    }

    pub fn handle_key(&mut self, scancode: u8) {
        if self.searching {
            match scancode {
                0x01 => {
                    self.searching = false;
                    self.search_len = 0;
                }
                0x0E => {
                    if self.search_len > 0 {
                        self.search_len -= 1;
                    }
                }
                0x1C => {
                    self.searching = false;
                }
                _ => {
                    if let Some(ch) = super::net_obs::scancode_to_char(scancode) {
                        if self.search_len < self.search_buf.len() {
                            self.search_buf[self.search_len] = ch;
                            self.search_len += 1;
                        }
                    }
                }
            }
        }
    }

    pub fn handle_click(&mut self, _px: i32, py: i32) {
        let row_h = (widgets::FONT_H + 4) as i32;
        let header = 60i32;
        let idx = ((py - header) / row_h).max(0) as usize;
        if idx < VISIBLE_ENTRIES {
            self.selected = idx + self.scroll_offset;
        }
    }
}

pub fn render(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;
    let arch = &app.archive;
    let changelog_len = app.changelog.len();

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;

    layout::draw_section(app, px, cy, "Archive of Echoes");
    cy += widgets::FONT_H + 4;

    // entry count
    let mut buf = [0u8; 8];
    let n = widgets::u64_to_str(changelog_len as u64, &mut buf);
    let count_str = core::str::from_utf8(&buf[..n]).unwrap_or("0");
    layout::draw_kv(app, px, cy, "Total entries:", count_str, t.telemetry);
    cy += widgets::FONT_H + 4;

    // search bar
    if arch.searching {
        let search_str = core::str::from_utf8(&arch.search_buf[..arch.search_len]).unwrap_or("");
        widgets::fill_rect(s, st, px, cy, 400, widgets::FONT_H + 4, t.input_bg, w, h);
        widgets::rect_outline(s, st, px, cy, 400, widgets::FONT_H + 4, t.signal, w, h);
        widgets::draw_str(s, st, px + 4, cy + 2, "Search: ", t.glyph_dim, t.input_bg, w, h);
        widgets::draw_str(s, st, px + 68, cy + 2, search_str, t.glyph, t.input_bg, w, h);
        let cursor_x = px + 68 + arch.search_len as u32 * widgets::FONT_W;
        widgets::fill_rect(s, st, cursor_x, cy + 2, 2, widgets::FONT_H, t.focus_ring, w, h);
    } else {
        widgets::draw_str(s, st, px, cy, "Press '/' or activate to search", t.glyph_dim, t.substrate, w, h);
    }
    cy += widgets::FONT_H + 8;

    // scroll indicators
    let can_up = arch.scroll_offset > 0;
    let can_down = changelog_len > arch.scroll_offset + VISIBLE_ENTRIES;
    let up_color = if can_up { t.glyph } else { t.contour };
    let dn_color = if can_down { t.glyph } else { t.contour };
    widgets::draw_str(s, st, px, cy, "^^ Up", up_color, t.substrate, w, h);
    widgets::draw_str(s, st, px + 80, cy, "vv Down", dn_color, t.substrate, w, h);
    cy += widgets::FONT_H + 4;

    // column header
    widgets::draw_str(s, st, px, cy, "#   Route          Field      Value", t.glyph_dim, t.substrate, w, h);
    cy += widgets::FONT_H + 2;
    widgets::hline(s, st, px, cy, (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD), t.contour, w, h);
    cy += 2;

    // entries
    let search_term = &arch.search_buf[..arch.search_len];
    let mut displayed = 0;
    let end = changelog_len.min(arch.scroll_offset + VISIBLE_ENTRIES);
    for i in arch.scroll_offset..end {
        let entry = &app.changelog[i];

        // search filter
        if arch.search_len > 0 {
            let field_bytes = entry.field_name.as_bytes();
            let value_bytes = &entry.description[..entry.desc_len];
            if !contains_subsequence(field_bytes, search_term)
                && !contains_subsequence(value_bytes, search_term)
            {
                continue;
            }
        }

        let is_selected = i == arch.selected;
        let row_bg = if is_selected { t.surface } else { t.substrate };

        let row_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
        widgets::fill_rect(s, st, px, cy, row_w, widgets::FONT_H + 2, row_bg, w, h);

        // index
        let mut nbuf = [0u8; 4];
        let nl = widgets::u64_to_str(i as u64, &mut nbuf);
        let ns = core::str::from_utf8(&nbuf[..nl]).unwrap_or("?");
        widgets::draw_str(s, st, px + 2, cy, ns, t.glyph_dim, row_bg, w, h);

        // route
        let route_str = entry.chamber.label();
        widgets::draw_str(s, st, px + 32, cy, route_str, t.glyph, row_bg, w, h);

        // field
        widgets::draw_str(s, st, px + 168, cy, entry.field_name, t.telemetry, row_bg, w, h);

        // value
        let val = core::str::from_utf8(&entry.description[..entry.desc_len]).unwrap_or("?");
        let val_color = if entry.destructive { t.destructive } else { t.success };
        widgets::draw_str_trunc(s, st, px + 260, cy, val, val_color, row_bg, w, h, row_w.saturating_sub(262) as usize);

        // destructive marker
        if entry.destructive {
            widgets::draw_str(s, st, px + row_w - 24, cy, "!!", t.destructive, row_bg, w, h);
        }

        cy += widgets::FONT_H + 2;
        displayed += 1;
    }

    if displayed == 0 {
        widgets::draw_str(s, st, px + 4, cy, "(no entries)", t.glyph_dim, t.substrate, w, h);
        cy += widgets::FONT_H + 2;
    }

    // detail panel for selected entry
    if arch.selected < changelog_len {
        cy += 8;
        widgets::hline(s, st, px, cy, (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD), t.contour, w, h);
        cy += 4;

        let entry = &app.changelog[arch.selected];
        layout::draw_section(app, px, cy, "Detail");
        cy += widgets::FONT_H + 4;

        layout::draw_kv(app, px, cy, "Chamber:", entry.chamber.label(), t.glyph);
        cy += widgets::FONT_H + 2;

        layout::draw_kv(app, px, cy, "Field:", entry.field_name, t.telemetry);
        cy += widgets::FONT_H + 2;

        let val = core::str::from_utf8(&entry.description[..entry.desc_len]).unwrap_or("?");
        let dc = if entry.destructive { t.destructive } else { t.success };
        layout::draw_kv(app, px, cy, "Value:", val, dc);
        cy += widgets::FONT_H + 2;

        if entry.destructive {
            layout::draw_risk_band(app, px, cy, "This was a destructive operation.");
        }
    }
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    // case-insensitive substring search
    let nlen = needle.len();
    for i in 0..=(haystack.len() - nlen) {
        let mut matched = true;
        for j in 0..nlen {
            let h = to_lower(haystack[i + j]);
            let n = to_lower(needle[j]);
            if h != n {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

#[inline(always)]
fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}
