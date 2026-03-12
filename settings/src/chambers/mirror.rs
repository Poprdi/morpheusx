// mirror basin — DE appearance controls.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{Route, SettingsApp};
use crate::theme::OneiricTheme;
use crate::widgets;
use libmorpheus::desktop::DesktopAppearance;

const FIELD_THEME_TOGGLE: usize = 0;
const FIELD_ACCENT_PREV: usize = 1;
const FIELD_ACCENT_NEXT: usize = 2;
const FIELD_APPLY: usize = 3;
const FIELD_REVERT: usize = 4;
const FIELD_COUNT: usize = 5;

const ACCENT_COUNT: usize = 6;
const ACCENTS: [(u8, u8, u8, &str); ACCENT_COUNT] = [
    (0, 230, 118, "Morpheus Green"),
    (100, 181, 246, "Dream Blue"),
    (255, 167, 38, "Oracle Amber"),
    (171, 71, 188, "Mist Violet"),
    (244, 67, 54, "Wrath Red"),
    (255, 255, 255, "Ghost White"),
];

pub struct MirrorChamber {
    pub dark_mode: bool,
    pub accent_idx: usize,
    pub saved_dark: bool,
    pub saved_accent: usize,
}

impl MirrorChamber {
    pub fn new() -> Self {
        Self {
            dark_mode: true,
            accent_idx: 0,
            saved_dark: true,
            saved_accent: 0,
        }
    }

    pub fn widget_count(&self) -> usize {
        FIELD_COUNT
    }

    pub fn apply(&mut self) {
        self.saved_dark = self.dark_mode;
        self.saved_accent = self.accent_idx;
    }

    pub fn revert(&mut self) {
        self.dark_mode = self.saved_dark;
        self.accent_idx = self.saved_accent;
    }

    pub fn restore_defaults(&mut self) {
        self.dark_mode = true;
        self.accent_idx = 0;
    }
}

pub fn sync_from_global(app: &mut SettingsApp) {
    if let Some(a) = DesktopAppearance::load() {
        app.mirror.dark_mode = a.dark_mode;
        app.mirror.accent_idx = nearest_accent_idx(a.accent_rgb);
        app.mirror.saved_dark = app.mirror.dark_mode;
        app.mirror.saved_accent = app.mirror.accent_idx;
        rebuild_theme(app);
    }
}

pub fn activate(app: &mut SettingsApp, idx: usize) {
    match idx {
        FIELD_THEME_TOGGLE => {
            app.mirror.dark_mode = !app.mirror.dark_mode;
            rebuild_theme(app);
            if let Err(code) = sync_global_from_local(app) {
                libmorpheus::println!("[settings/mirror] de appearance sync failed err=0x{:x}", code);
                app.set_status("DE appearance sync failed", true);
            }
            app.mark_edited(Route::MirrorBasin, "theme_mode");
        }
        FIELD_ACCENT_PREV => {
            app.mirror.accent_idx = if app.mirror.accent_idx == 0 {
                ACCENT_COUNT - 1
            } else {
                app.mirror.accent_idx - 1
            };
            rebuild_theme(app);
            if let Err(code) = sync_global_from_local(app) {
                libmorpheus::println!("[settings/mirror] de appearance sync failed err=0x{:x}", code);
                app.set_status("DE appearance sync failed", true);
            }
            app.mark_edited(Route::MirrorBasin, "accent");
        }
        FIELD_ACCENT_NEXT => {
            app.mirror.accent_idx = (app.mirror.accent_idx + 1) % ACCENT_COUNT;
            rebuild_theme(app);
            if let Err(code) = sync_global_from_local(app) {
                libmorpheus::println!("[settings/mirror] de appearance sync failed err=0x{:x}", code);
                app.set_status("DE appearance sync failed", true);
            }
            app.mark_edited(Route::MirrorBasin, "accent");
        }
        FIELD_APPLY => {
            let accent_name = ACCENTS[app.mirror.accent_idx].3;
            match sync_global_from_local(app) {
                Ok(()) => {
                    app.mirror.apply();
                    app.clear_pending_for(Route::MirrorBasin);
                    app.set_status("DE appearance applied", false);
                    app.log_change(Route::MirrorBasin, "appearance", accent_name, false);
                }
                Err(code) => {
                    libmorpheus::println!("[settings/mirror] de appearance apply failed err=0x{:x}", code);
                    app.set_status("DE appearance apply failed", true);
                }
            }
        }
        FIELD_REVERT => {
            app.mirror.revert();
            rebuild_theme(app);
            let _ = sync_global_from_local(app);
            app.clear_pending_for(Route::MirrorBasin);
            app.set_status("Appearance reverted", false);
        }
        _ => {}
    }
}

fn rebuild_theme(app: &mut SettingsApp) {
    let dark = app.mirror.dark_mode;
    let ai = app.mirror.accent_idx;
    let base = if dark {
        OneiricTheme::dark()
    } else {
        OneiricTheme::light()
    };
    let (r, g, b, _) = ACCENTS[ai];
    let accent = crate::theme::pack(r, g, b);
    app.theme.signal = accent;
    app.theme.focus_ring = accent;
    app.theme.rail_active = accent;
    app.theme.substrate = base.substrate;
    app.theme.contour = base.contour;
    app.theme.glyph = base.glyph;
    app.theme.glyph_dim = base.glyph_dim;
    app.theme.surface = base.surface;
    app.theme.input_bg = base.input_bg;
    app.theme.rail_bg = base.rail_bg;
    app.theme.bar_bg = base.bar_bg;
    app.theme.strip_bg = base.strip_bg;
    app.frame_dirty = true;
}

fn nearest_accent_idx(target: (u8, u8, u8)) -> usize {
    let mut best = 0usize;
    let mut best_dist = u32::MAX;
    for (i, (r, g, b, _)) in ACCENTS.iter().enumerate() {
        let dr = (*r as i32 - target.0 as i32) as i64;
        let dg = (*g as i32 - target.1 as i32) as i64;
        let db = (*b as i32 - target.2 as i32) as i64;
        let d = (dr * dr + dg * dg + db * db) as u32;
        if d < best_dist {
            best_dist = d;
            best = i;
        }
    }
    best
}

fn sync_global_from_local(app: &SettingsApp) -> Result<(), u64> {
    let (r, g, b, _) = ACCENTS[app.mirror.accent_idx];
    let profile = DesktopAppearance::from_theme_choice(app.mirror.dark_mode, (r, g, b));
    profile.store()
}

pub fn handle_key(_app: &mut SettingsApp, _scancode: u8) {}

pub fn render(app: &SettingsApp) {
    let t = &app.theme;
    let mirror = &app.mirror;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;
    let r2 = layout::row_step(app, 2);
    let r4 = layout::row_step(app, 4);
    let r8 = layout::row_step(app, 8);
    let r12 = layout::row_step(app, 12);

    layout::draw_section(app, px, cy, "Theme Mode");
    cy += r4;

    let mode_label = if mirror.dark_mode {
        "[X] Dark   [ ] Light"
    } else {
        "[ ] Dark   [X] Light"
    };
    layout::draw_button_row(app, px, cy, mode_label, FIELD_THEME_TOGGLE, t.glyph);
    cy += r8;

    layout::draw_section(app, px, cy, "Accent Color");
    cy += r4;

    let (ar, ag, ab, name) = ACCENTS[mirror.accent_idx];
    let accent = crate::theme::pack(ar, ag, ab);

    for i in 0..ACCENT_COUNT {
        let (r, g, b, _) = ACCENTS[i];
        let c = crate::theme::pack(r, g, b);
        let sx = px + i as u32 * 28;
        let swatch_y = cy;
        widgets::fill_rect(app.surface, app.fb_stride, sx, swatch_y, 24, 16, c, app.fb_w, app.fb_h);
        if i == mirror.accent_idx {
            widgets::rect_outline(
                app.surface,
                app.fb_stride,
                sx.saturating_sub(1),
                swatch_y.saturating_sub(1),
                26,
                18,
                t.focus_ring,
                app.fb_w,
                app.fb_h,
            );
        }
    }
    cy += 20;

    widgets::draw_str(app.surface, app.fb_stride, px, cy, name, accent, t.substrate, app.fb_w, app.fb_h);
    cy += r4;

    layout::draw_button_row(app, px, cy, "<< Previous Accent", FIELD_ACCENT_PREV, t.glyph);
    cy += r4;
    layout::draw_button_row(app, px, cy, ">> Next Accent", FIELD_ACCENT_NEXT, t.glyph);
    cy += r12;

    layout::draw_section(app, px, cy, "Preview");
    cy += r4;

    let tokens: [(&str, u32); 10] = [
        ("substrate", t.substrate),
        ("glyph", t.glyph),
        ("signal", t.signal),
        ("warning", t.warning),
        ("destructive", t.destructive),
        ("immutable", t.immutable),
        ("telemetry", t.telemetry),
        ("success", t.success),
        ("focus_ring", t.focus_ring),
        ("armed", t.armed),
    ];

    for (name, color) in &tokens {
        widgets::fill_rect(app.surface, app.fb_stride, px, cy, 16, 12, *color, app.fb_w, app.fb_h);
        widgets::draw_str(app.surface, app.fb_stride, px + 20, cy, name, t.glyph, t.substrate, app.fb_w, app.fb_h);
        cy += r2;
    }

    cy += 8;

    layout::draw_button_row(app, px, cy, "Apply Appearance", FIELD_APPLY, t.signal);
    cy += r4;
    layout::draw_button_row(app, px, cy, "Revert", FIELD_REVERT, t.glyph_dim);
}
