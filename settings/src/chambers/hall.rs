// hall of masks — preset bundles.
// named theme+config presets with delta preview before apply.
// "Default Dark", "Default Light", custom combos. snapshot → preview → apply.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{Route, SettingsApp};
use crate::theme::{self, OneiricTheme};
use crate::widgets;

const PRESET_COUNT: usize = 4;

struct Preset {
    name: &'static str,
    desc: &'static str,
    dark: bool,
    accent_r: u8,
    accent_g: u8,
    accent_b: u8,
}

const PRESETS: [Preset; PRESET_COUNT] = [
    Preset {
        name: "Default Dark",
        desc: "The canonical 3am terminal aesthetic. Green on black.",
        dark: true,
        accent_r: 0,
        accent_g: 230,
        accent_b: 118,
    },
    Preset {
        name: "Default Light",
        desc: "Inverted substrate. For people who work with the lights on.",
        dark: false,
        accent_r: 0,
        accent_g: 150,
        accent_b: 80,
    },
    Preset {
        name: "Dream Protocol",
        desc: "Blue-shifted. Calm. The sleep cycle palette.",
        dark: true,
        accent_r: 100,
        accent_g: 181,
        accent_b: 246,
    },
    Preset {
        name: "Oracle Fire",
        desc: "Amber warnings. For those who like to see the future burn.",
        dark: true,
        accent_r: 255,
        accent_g: 167,
        accent_b: 38,
    },
];

const FIELD_APPLY: usize = PRESET_COUNT;
const FIELD_COUNT: usize = PRESET_COUNT + 1;

pub struct HallChamber {
    pub selected: usize,
    pub preview_active: bool,
}

impl HallChamber {
    pub fn new() -> Self {
        Self {
            selected: 0,
            preview_active: false,
        }
    }

    pub fn widget_count(&self) -> usize {
        FIELD_COUNT
    }

    pub fn revert(&mut self) {
        self.preview_active = false;
    }

    pub fn restore_defaults(&mut self) {
        self.selected = 0;
        self.preview_active = false;
    }
}

pub fn activate(app: &mut SettingsApp, idx: usize) {
    if idx < PRESET_COUNT {
        app.hall.selected = idx;
        app.hall.preview_active = true;
        apply_theme_preview(app);
        app.set_status("Previewing preset. Enter on Apply to commit.", false);
    } else if idx == FIELD_APPLY {
        apply(app);
    }
}

fn apply_theme_preview(app: &mut SettingsApp) {
    let sel = app.hall.selected;
    let p = &PRESETS[sel];
    let base = if p.dark { OneiricTheme::dark() } else { OneiricTheme::light() };
    let accent = theme::pack(p.accent_r, p.accent_g, p.accent_b);

    app.theme = base;
    app.theme.signal = accent;
    app.theme.focus_ring = accent;
    app.theme.rail_active = accent;

    app.mirror.dark_mode = p.dark;
    app.mirror.accent_idx = 0;
    app.frame_dirty = true;
}

pub fn apply(app: &mut SettingsApp) {
    if !app.hall.preview_active {
        app.set_status("Select a preset first", false);
        return;
    }
    apply_theme_preview(app);
    app.mirror.apply();
    app.hall.preview_active = false;
    let name = PRESETS[app.hall.selected].name;
    app.log_change(Route::HallOfMasks, "preset", name, false);
    app.set_status("Preset applied", false);
}

pub fn handle_key(_app: &mut SettingsApp, _scancode: u8) {}

pub fn handle_click(app: &mut SettingsApp, _px: i32, py: i32) {
    let row_h = (widgets::FONT_H + 12) as i32;
    let header = 60i32;
    let idx = ((py - header) / row_h).max(0) as usize;
    if idx < FIELD_COUNT {
        app.pane_focus = idx;
        activate(app, idx);
    }
}

pub fn render(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;
    let hall = &app.hall;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;
    let r2 = layout::row_step(app, 2);
    let r4 = layout::row_step(app, 4);
    let r8 = layout::row_step(app, 8);

    layout::draw_section(app, px, cy, "Profiles");
    cy += r4;

    widgets::draw_str(s, st, px, cy, "Select a preset to preview, then Apply to commit.", t.glyph_dim, t.substrate, w, h);
    cy += r8;

    // preset cards
    for i in 0..PRESET_COUNT {
        let p = &PRESETS[i];
        let is_selected = i == hall.selected;
        let is_focused = !app.focus_in_rail && app.pane_focus == i;

        let card_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
        let card_h = widgets::FONT_H * 2 + 12;
        let card_bg = if is_selected { t.surface } else { t.substrate };

        widgets::fill_rect(s, st, px, cy, card_w, card_h, card_bg, w, h);

        let border = if is_focused {
            t.focus_ring
        } else if is_selected {
            t.signal
        } else {
            t.contour
        };
        widgets::rect_outline(s, st, px, cy, card_w, card_h, border, w, h);

        // accent swatch
        let accent = theme::pack(p.accent_r, p.accent_g, p.accent_b);
        widgets::fill_rect(s, st, px + 4, cy + 4, 16, 16, accent, w, h);

        // name
        let name_color = if is_selected { t.signal } else { t.glyph };
        widgets::draw_str(s, st, px + 24, cy + 4, p.name, name_color, card_bg, w, h);

        // description
        widgets::draw_str(s, st, px + 24, cy + 4 + widgets::FONT_H + 2, p.desc, t.glyph_dim, card_bg, w, h);

        // selection marker
        if is_selected {
            widgets::draw_str(s, st, px + card_w - 28, cy + 4, ">>", t.signal, card_bg, w, h);
        }

        cy += card_h + 4;
    }

    cy += 8;

    // delta preview
    if hall.preview_active {
        layout::draw_section(app, px, cy, "Delta Preview");
        cy += r4;

        let p = &PRESETS[hall.selected];
        let mode_str = if p.dark { "Dark" } else { "Light" };
        layout::draw_kv(app, px, cy, "Mode:", mode_str, t.glyph);
        cy += r2;

        // show accent color value
        let accent = theme::pack(p.accent_r, p.accent_g, p.accent_b);
        widgets::fill_rect(s, st, px + 80, cy, 48, 12, accent, w, h);
        widgets::draw_str(s, st, px, cy, "Accent:", t.glyph, t.substrate, w, h);
        cy += r2;

        layout::draw_kv(app, px, cy, "Name:", p.name, t.immutable);
        cy += r8;
    }

    // apply button
    let apply_label = if hall.preview_active { "Apply This Mask" } else { "Select a Mask First" };
    let apply_color = if hall.preview_active { t.signal } else { t.glyph_dim };
    layout::draw_button_row(app, px, cy, apply_label, FIELD_APPLY, apply_color);
}
