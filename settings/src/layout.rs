// global frame layout — the three-column oneiric structure.
// top strip, left rail, right console pane, bottom command bar.
// every pixel accounted for, every margin deliberate.

use crate::state::{Route, SafetyMode, SettingsApp};

use crate::widgets;

pub const STRIP_HEIGHT: u32 = 24;
pub const BAR_HEIGHT: u32 = 24;
pub const RAIL_WIDTH: u32 = 160;
pub const RAIL_ITEM_HEIGHT: u32 = 24;
pub const SECTION_PAD: u32 = 8;
pub const PANE_PAD: u32 = 12;

#[inline(always)]
pub fn row_step(app: &SettingsApp, extra: u32) -> u32 {
    let pane_h = app
        .fb_h
        .saturating_sub(STRIP_HEIGHT + BAR_HEIGHT + 2 * PANE_PAD);
    // grow spacing on taller panes so content doesn't collapse into the top third
    let bonus = (pane_h / 120).saturating_sub(4).min(8);
    widgets::FONT_H + extra + bonus
}

pub fn render_frame(app: &mut SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    // clear entire surface
    widgets::fill_rect(s, st, 0, 0, w, h, t.substrate, w, h);

    // top command strip
    render_strip(app);

    // left rail
    render_rail(app);

    // right console pane
    render_pane(app);

    // bottom command bar
    render_bar(app);
}

fn render_strip(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    widgets::fill_rect(s, st, 0, 0, w, STRIP_HEIGHT, t.strip_bg, w, h);
    widgets::hline(s, st, 0, STRIP_HEIGHT - 1, w, t.contour, w, h);

    widgets::draw_str(s, st, 4, 4, "Settings", t.glyph_dim, t.strip_bg, w, h);
    let label = app.route.label();
    let title_x = 4 + 10 * widgets::FONT_W;
    widgets::draw_str(s, st, title_x, 4, label, t.glyph, t.strip_bg, w, h);

    // mode badge
    let (badge, badge_color) = match app.safety {
        SafetyMode::Safe => ("SAFE", t.signal),
        SafetyMode::Severe => ("SEVERE", t.destructive),
    };
    let badge_x = w.saturating_sub((badge.len() as u32 + 2) * widgets::FONT_W);
    widgets::draw_str(s, st, badge_x, 2, badge, badge_color, t.strip_bg, w, h);

    // pending indicator
    if app.has_any_pending() {
        let dot_x = badge_x.saturating_sub(3 * widgets::FONT_W);
        widgets::draw_str(s, st, dot_x, 2, "[*]", t.warning, t.strip_bg, w, h);
    }
}

fn render_rail(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let rail_h = h - STRIP_HEIGHT - BAR_HEIGHT;
    widgets::fill_rect(s, st, 0, STRIP_HEIGHT, RAIL_WIDTH, rail_h, t.rail_bg, w, h);
    widgets::vline(s, st, RAIL_WIDTH - 1, STRIP_HEIGHT, rail_h, t.contour, w, h);

    for (i, route) in Route::ALL.iter().enumerate() {
        let y = STRIP_HEIGHT + i as u32 * RAIL_ITEM_HEIGHT;
        let is_current = *route == app.route;
        let is_focused = app.focus_in_rail && app.rail_focus == i;

        let bg = if is_current {
            t.rail_active
        } else if is_focused {
            t.surface
        } else {
            t.rail_bg
        };

        widgets::fill_rect(s, st, 0, y, RAIL_WIDTH - 1, RAIL_ITEM_HEIGHT, bg, w, h);

        // focus ring
        if is_focused {
            widgets::rect_outline(s, st, 0, y, RAIL_WIDTH - 1, RAIL_ITEM_HEIGHT, t.focus_ring, w, h);
        }

        // sigil + label
        let sigil = route.sigil();
        let label = route.label();
        let fg = if is_current { t.glyph } else { t.glyph_dim };
        widgets::draw_str(s, st, 4, y + 2, sigil, t.signal, bg, w, h);
        widgets::draw_str_trunc(s, st, 4 + 2 * widgets::FONT_W, y + 4, label, fg, bg, w, h, 14);

        // keyboard hint (1-7)
        let mut hint = [0u8; 1];
        hint[0] = b'1' + i as u8;
        let hint_str = core::str::from_utf8(&hint).unwrap_or("?");
        let hint_x = RAIL_WIDTH - 3 * widgets::FONT_W;
        widgets::draw_str(s, st, hint_x, y + 4, hint_str, t.glyph_dim, bg, w, h);

        // pending dot for this chamber
        if app.has_pending_for(*route) {
            let dot_x = RAIL_WIDTH - 5 * widgets::FONT_W;
            widgets::draw_str(s, st, dot_x, y + 4, "*", t.warning, bg, w, h);
        }
    }
}

fn render_pane(app: &mut SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let pane_x = RAIL_WIDTH;
    let pane_y = STRIP_HEIGHT;
    let pane_w = w - RAIL_WIDTH;
    let pane_h = h - STRIP_HEIGHT - BAR_HEIGHT;

    widgets::fill_rect(s, st, pane_x, pane_y, pane_w, pane_h, t.substrate, w, h);

    // dispatch to the active chamber renderer
    match app.route {
        Route::Gateway => crate::chambers::gateway::render(app),
        Route::MistShore => crate::chambers::mist::render(app),
        Route::MirrorBasin => crate::chambers::mirror::render(app),
        Route::NetObservatory => crate::chambers::net_obs::render(app),
        Route::SysObservatory => crate::chambers::sys_obs::render(app),
        Route::Archive => crate::chambers::archive::render(app),
        Route::HallOfMasks => crate::chambers::hall::render(app),
    }
}

fn render_bar(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let bar_y = h - BAR_HEIGHT;
    widgets::fill_rect(s, st, 0, bar_y, w, BAR_HEIGHT, t.bar_bg, w, h);
    widgets::hline(s, st, 0, bar_y, w, t.contour, w, h);

    let pane_w = w - RAIL_WIDTH;
    let btn_w = pane_w / 4;
    let btn_y = bar_y + 2;

    // command buttons
    let buttons = ["[A]pply", "[R]evert", "[D]efaults", ""];
    for (i, label) in buttons.iter().enumerate() {
        if label.is_empty() {
            continue;
        }
        let bx = RAIL_WIDTH + i as u32 * btn_w;
        let fg = if i == 0 && app.has_any_pending() {
            t.signal
        } else {
            t.glyph_dim
        };
        widgets::draw_str(s, st, bx + 4, btn_y, label, fg, t.bar_bg, w, h);
    }

    // status message (right-aligned in bar)
    if app.status_len > 0 {
        let msg = core::str::from_utf8(&app.status_msg[..app.status_len]).unwrap_or("");
        let fg = if app.status_is_error { t.destructive } else { t.success };
        let sx = w.saturating_sub((msg.len() as u32 + 1) * widgets::FONT_W);
        widgets::draw_str(s, st, sx, btn_y, msg, fg, t.bar_bg, w, h);
    }
}

// section header — a labeled divider within the pane
pub fn draw_section(app: &SettingsApp, x: u32, y: u32, title: &str) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    widgets::draw_str(s, st, x, y, title, t.signal, t.substrate, w, h);
    let line_x = x + (title.len() as u32 + 1) * widgets::FONT_W;
    let line_w = (w - RAIL_WIDTH).saturating_sub(line_x - RAIL_WIDTH + PANE_PAD);
    widgets::hline(s, st, line_x, y + widgets::FONT_H / 2, line_w, t.contour, w, h);
}

// labeled value row — "Label: value" with alignment
pub fn draw_kv(app: &SettingsApp, x: u32, y: u32, key: &str, val: &str, val_color: u32) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    widgets::draw_str(s, st, x, y, key, t.glyph_dim, t.substrate, w, h);
    let vx = x + (key.len() as u32 + 1) * widgets::FONT_W;
    let pane_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let max_chars = pane_w.saturating_sub(vx.saturating_sub(x)) / widgets::FONT_W;
    widgets::draw_str_trunc(s, st, vx, y, val, val_color, t.substrate, w, h, max_chars as usize);
}

// focusable field row — highlighted when focused
pub fn draw_field_row(app: &SettingsApp, x: u32, y: u32, label: &str, value: &str, _focused: bool, field_idx: usize) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let is_focused = !app.focus_in_rail && app.pane_focus == field_idx;

    let bg = if is_focused { t.surface } else { t.substrate };
    let row_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let row_h = row_step(app, 4);
    app.register_widget_hitbox(x, y, row_w, row_h, field_idx);
    widgets::fill_rect(s, st, x, y, row_w, row_h, bg, w, h);

    if is_focused {
        widgets::rect_outline(s, st, x, y, row_w, row_h, t.focus_ring, w, h);
    }

    let ty = y + row_h.saturating_sub(widgets::FONT_H) / 2;

    let label_w = (row_w / 3).clamp(12 * widgets::FONT_W, 22 * widgets::FONT_W);
    let label_chars = label_w.saturating_sub(8) / widgets::FONT_W;
    widgets::draw_str_trunc(s, st, x + 4, ty, label, t.glyph_dim, bg, w, h, label_chars as usize);
    let vx = x + label_w;
    let value_chars = row_w.saturating_sub(label_w + 6) / widgets::FONT_W;
    widgets::draw_str_trunc(s, st, vx, ty, value, t.glyph, bg, w, h, value_chars as usize);
}

// button row — rendered as a text button
pub fn draw_button_row(app: &SettingsApp, x: u32, y: u32, label: &str, field_idx: usize, color: u32) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let is_focused = !app.focus_in_rail && app.pane_focus == field_idx;

    let bg = if is_focused { t.surface } else { t.substrate };
    let btn_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let btn_h = row_step(app, 4);
    app.register_widget_hitbox(x, y, btn_w, btn_h, field_idx);

    widgets::fill_rect(s, st, x, y, btn_w, btn_h, bg, w, h);
    widgets::rect_outline(s, st, x, y, btn_w, btn_h, if is_focused { t.focus_ring } else { t.contour }, w, h);
    let label_chars = btn_w.saturating_sub(4 * widgets::FONT_W) / widgets::FONT_W;
    let ty = y + btn_h.saturating_sub(widgets::FONT_H) / 2;
    widgets::draw_str_trunc(s, st, x + 2 * widgets::FONT_W, ty, label, color, bg, w, h, label_chars as usize);
}

// risk band — a warning banner for destructive context
pub fn draw_risk_band(app: &SettingsApp, x: u32, y: u32, msg: &str) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let band_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let band_h = row_step(app, 8);
    widgets::fill_rect(s, st, x, y, band_w, band_h, t.destructive, w, h);
    let ty = y + band_h.saturating_sub(widgets::FONT_H) / 2;
    widgets::draw_str_trunc(
        s,
        st,
        x + 4,
        ty,
        msg,
        t.substrate,
        t.destructive,
        w,
        h,
        band_w.saturating_sub(8) as usize / widgets::FONT_W as usize,
    );
}
