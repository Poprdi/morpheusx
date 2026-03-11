// oneiric gateway — the entry chamber.
// safe/severe mode gate, quick-jump links, last-visited recall.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{ArmState, Route, SafetyMode, SettingsApp};
use crate::widgets;

pub struct GatewayChamber {
    pub recent: [Route; 3],
    pub recent_count: usize,
}

impl GatewayChamber {
    pub fn new() -> Self {
        Self {
            recent: [Route::Gateway; 3],
            recent_count: 0,
        }
    }

    pub fn record_visit(&mut self, route: Route) {
        if route == Route::Gateway {
            return;
        }
        let mut new = [Route::Gateway; 3];
        new[0] = route;
        let mut n = 1;
        for i in 0..self.recent_count {
            if self.recent[i] != route && n < 3 {
                new[n] = self.recent[i];
                n += 1;
            }
        }
        self.recent = new;
        self.recent_count = n;
    }

    pub fn widget_count(&self) -> usize {
        1 + Route::ALL.len() + self.recent_count
    }
}

pub fn activate(app: &mut SettingsApp, idx: usize) {
    if idx == 0 {
        match app.safety {
            SafetyMode::Safe => {
                if app.severe_arm == ArmState::Disarmed {
                    app.severe_arm = ArmState::Armed;
                    app.set_status("Severe mode ARMED — press Enter to confirm, Esc to disarm", true);
                } else if app.severe_arm == ArmState::Armed {
                    app.safety = SafetyMode::Severe;
                    app.severe_arm = ArmState::Confirmed;
                    app.set_status("Severe mode ACTIVE", true);
                }
            }
            SafetyMode::Severe => {
                app.safety = SafetyMode::Safe;
                app.severe_arm = ArmState::Disarmed;
                app.set_status("Safe mode restored", false);
            }
        }
    } else if idx <= Route::ALL.len() {
        let target = Route::from_index(idx - 1);
        let cur = app.route;
        app.gateway.record_visit(cur);
        app.navigate(target);
    } else {
        let ri = idx - 1 - Route::ALL.len();
        if ri < app.gateway.recent_count {
            let target = app.gateway.recent[ri];
            app.navigate(target);
        }
    }
}

pub fn handle_key(app: &mut SettingsApp, scancode: u8) {
    let num = match scancode {
        0x02 => Some(0),
        0x03 => Some(1),
        0x04 => Some(2),
        0x05 => Some(3),
        0x06 => Some(4),
        0x07 => Some(5),
        0x08 => Some(6),
        _ => None,
    };
    if let Some(i) = num {
        if i < Route::ALL.len() {
            let cur = app.route;
            app.gateway.record_visit(cur);
            app.navigate(Route::from_index(i));
        }
    }
}

pub fn handle_click(app: &mut SettingsApp, _px: i32, py: i32) {
    let row_h = layout::row_step(app, 8);
    let idx = (py as u32 / row_h) as usize;
    if idx >= 2 {
        let adjusted = idx - 2;
        app.pane_focus = adjusted;
        activate(app, adjusted);
    }
}

pub fn render(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;
    let r4 = layout::row_step(app, 4);
    let r8 = layout::row_step(app, 8);
    let r12 = layout::row_step(app, 12);

    // welcome header
    widgets::draw_str(s, st, px, cy, "General Settings", t.signal, t.substrate, w, h);
    cy += r4;
    widgets::draw_str(s, st, px, cy, "System configuration interface", t.glyph_dim, t.substrate, w, h);
    cy += r12;

    // mode gate
    layout::draw_section(app, px, cy, "Mode");
    cy += r4;

    let (mode_label, mode_color) = match app.safety {
        SafetyMode::Safe => ("[ ] Enter Severe Mode", t.glyph_dim),
        SafetyMode::Severe => ("[X] Severe Mode ACTIVE", t.destructive),
    };
    layout::draw_button_row(app, px, cy, mode_label, 0, mode_color);
    cy += r8;

    // armed warning
    if app.severe_arm == ArmState::Armed {
        layout::draw_risk_band(app, px, cy, "WARNING: Severe mode unlocks destructive system controls. Press Enter to confirm.");
        cy += r12;
    }

    // chamber links
    layout::draw_section(app, px, cy, "Sections");
    cy += r4;

    for (i, route) in Route::ALL.iter().enumerate() {
        let label = route.label();
        let sigil = route.sigil();
        let is_focused = !app.focus_in_rail && app.pane_focus == i + 1;

        let bg = if is_focused { t.surface } else { t.substrate };
        let fg = if is_focused { t.signal } else { t.glyph };

        let row_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
        widgets::fill_rect(s, st, px, cy, row_w, r4, bg, w, h);
        if is_focused {
            widgets::rect_outline(s, st, px, cy, row_w, r4, t.focus_ring, w, h);
        }

        let ty = cy + r4.saturating_sub(widgets::FONT_H) / 2;

        // number key hint
        let mut num_buf = [0u8; 1];
        num_buf[0] = b'1' + i as u8;
        let num_str = core::str::from_utf8(&num_buf).unwrap_or("?");
        widgets::draw_str(s, st, px + 4, ty, num_str, t.glyph_dim, bg, w, h);

        // sigil
        widgets::draw_str(s, st, px + 3 * widgets::FONT_W, ty, sigil, t.signal, bg, w, h);

        // label
        widgets::draw_str(s, st, px + 5 * widgets::FONT_W, ty, label, fg, bg, w, h);

        cy += r4;
    }

    // recent chambers
    if app.gateway.recent_count > 0 {
        cy += 8;
        layout::draw_section(app, px, cy, "Recent");
        cy += r4;

        for i in 0..app.gateway.recent_count {
            let route = app.gateway.recent[i];
            let field_idx = 1 + Route::ALL.len() + i;
            let label = route.label();
            layout::draw_button_row(app, px, cy, label, field_idx, t.glyph);
            cy += r8;
        }
    }
}
