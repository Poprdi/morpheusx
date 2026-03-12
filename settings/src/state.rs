use alloc::vec::Vec;


use crate::chambers::{
    archive::ArchiveChamber, gateway::GatewayChamber, hall::HallChamber,
    mirror::MirrorChamber, mist::MistChamber, net_obs::NetObsChamber,
    sys_obs::SysObsChamber,
};
use crate::layout;
use crate::theme::OneiricTheme;
use crate::widgets;

// route ids — flat enum, zero-cost dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Route {
    Gateway = 0,
    MistShore = 1,
    MirrorBasin = 2,
    NetObservatory = 3,
    SysObservatory = 4,
    Archive = 5,
    HallOfMasks = 6,
}

impl Route {
    pub const ALL: [Route; 7] = [
        Route::Gateway,
        Route::MistShore,
        Route::MirrorBasin,
        Route::NetObservatory,
        Route::SysObservatory,
        Route::Archive,
        Route::HallOfMasks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Route::Gateway => "General",
            Route::MistShore => "Display",
            Route::MirrorBasin => "Appearance",
            Route::NetObservatory => "Network",
            Route::SysObservatory => "System",
            Route::Archive => "Activity",
            Route::HallOfMasks => "Profiles",
        }
    }

    pub fn sigil(self) -> &'static str {
        match self {
            Route::Gateway => "G",
            Route::MistShore => "D",
            Route::MirrorBasin => "A",
            Route::NetObservatory => "N",
            Route::SysObservatory => "S",
            Route::Archive => "L",
            Route::HallOfMasks => "P",
        }
    }

    pub fn technical_alias(self) -> &'static str {
        match self {
            Route::Gateway => "/gateway",
            Route::MistShore => "/visual/mist-shore",
            Route::MirrorBasin => "/visual/mirror-basin",
            Route::NetObservatory => "/network/observatory",
            Route::SysObservatory => "/system/observatory",
            Route::Archive => "/storage/archive",
            Route::HallOfMasks => "/presets/hall-of-masks",
        }
    }

    pub fn from_index(i: usize) -> Route {
        Route::ALL[i.min(Route::ALL.len() - 1)]
    }
}

// setting lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FieldState {
    Pristine = 0,
    Edited = 1,
    Staged = 2,
    Applied = 3,
    Failed = 4,
}

// safety mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SafetyMode {
    Safe = 0,
    Severe = 1,
}

// armed confirmation for destructive actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ArmState {
    Disarmed = 0,
    Armed = 1,
    Confirmed = 2,
}

// pending change tracker
pub struct PendingChange {
    pub chamber: Route,
    pub field_name: &'static str,
    pub state: FieldState,
}

// changelog entry for Archive of Echoes
pub struct ChangeEntry {
    pub timestamp_ms: u64,
    pub chamber: Route,
    pub field_name: &'static str,
    pub description: [u8; 128],
    pub desc_len: usize,
    pub destructive: bool,
}

#[derive(Clone, Copy)]
pub struct Hitbox {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub widget_idx: usize,
}

// the master state machine
pub struct SettingsApp {
    // display surface
    pub surface: *mut u32,
    pub fb_w: u32,
    pub fb_h: u32,
    pub fb_stride: u32,
    pub is_bgrx: bool,

    // navigation
    pub route: Route,
    pub prev_route: Route,
    pub rail_focus: usize,
    pub pane_focus: usize,
    pub focus_in_rail: bool,

    // mode
    pub safety: SafetyMode,
    pub severe_arm: ArmState,

    // theme
    pub theme: OneiricTheme,

    // dirty tracking
    pub pending: Vec<PendingChange>,
    pub changelog: Vec<ChangeEntry>,
    pub frame_dirty: bool,
    pub status_msg: [u8; 128],
    pub status_len: usize,
    pub status_is_error: bool,

    // mouse state
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub last_buttons: u8,

    // chamber states
    pub gateway: GatewayChamber,
    pub mist: MistChamber,
    pub mirror: MirrorChamber,
    pub net_obs: NetObsChamber,
    pub sys_obs: SysObsChamber,
    pub archive: ArchiveChamber,
    pub hall: HallChamber,

    // tick counter for animations
    pub tick_count: u64,

    pub hitboxes: [Hitbox; 128],
    pub hitbox_count: usize,
}

impl SettingsApp {
    pub fn new(
        surface: *mut u32,
        fb_w: u32,
        fb_h: u32,
        fb_stride: u32,
        is_bgrx: bool,
    ) -> Self {
        Self {
            surface,
            fb_w,
            fb_h,
            fb_stride,
            is_bgrx,

            route: Route::Gateway,
            prev_route: Route::Gateway,
            rail_focus: 0,
            pane_focus: 0,
            focus_in_rail: true,

            safety: SafetyMode::Safe,
            severe_arm: ArmState::Disarmed,

            theme: OneiricTheme::dark(),

            pending: Vec::new(),
            changelog: Vec::new(),
            frame_dirty: true,
            status_msg: [0; 128],
            status_len: 0,
            status_is_error: false,

            mouse_x: 0,
            mouse_y: 0,
            last_buttons: 0,

            gateway: GatewayChamber::new(),
            mist: MistChamber::new(),
            mirror: MirrorChamber::new(),
            net_obs: NetObsChamber::new(),
            sys_obs: SysObsChamber::new(),
            archive: ArchiveChamber::new(),
            hall: HallChamber::new(),

            tick_count: 0,

            hitboxes: [Hitbox {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                widget_idx: 0,
            }; 128],
            hitbox_count: 0,
        }
    }

    pub fn init(&mut self) {
        self.frame_dirty = true;
        self.net_obs.refresh();
        self.sys_obs.refresh();
        self.mist.refresh();
        self.set_status("Tab switch focus, W/S move, Enter activate, 1-7 jump sections", false);
    }

    pub fn tick(&mut self) {
        self.tick_count += 1;

        // poll input from compositor
        self.poll_input();

        // periodic refresh for observatory chambers (every ~60 ticks ≈ 1 second)
        if self.tick_count % 60 == 0 {
            match self.route {
                Route::SysObservatory => self.sys_obs.refresh(),
                Route::NetObservatory => self.net_obs.refresh(),
                _ => {}
            }
            self.frame_dirty = true;
        }

        if self.frame_dirty {
            self.render();
            self.frame_dirty = false;
        }
    }

    fn poll_input(&mut self) {
        // keyboard
        let avail = libmorpheus::io::stdin_available();
        if avail > 0 {
            let mut buf = [0u8; 1];
            let n = libmorpheus::io::read_stdin(&mut buf);
            if n > 0 {
                self.handle_key(buf[0]);
            }
        }

        // mouse
        let ms = libmorpheus::hw::mouse_read();
        if ms.dx != 0 || ms.dy != 0 || ms.buttons != 0 {
            self.mouse_x = (self.mouse_x + ms.dx as i32).clamp(0, self.fb_w as i32 - 1);
            self.mouse_y = (self.mouse_y + ms.dy as i32).clamp(0, self.fb_h as i32 - 1);

            let left = (ms.buttons & 1) != 0;
            let left_was = (self.last_buttons & 1) != 0;
            if left && !left_was {
                self.handle_click(self.mouse_x, self.mouse_y);
            }
            self.last_buttons = ms.buttons;
            self.frame_dirty = true;
        }
    }

    fn handle_key(&mut self, key: u8) {
        self.frame_dirty = true;

        if matches!(key, b'1'..=b'7') {
            let idx = (key - b'1') as usize;
            self.rail_focus = idx;
            self.navigate(Route::from_index(idx));
            return;
        }

        match key {
            // Tab = toggle rail/pane focus
            0x0F | b'\t' => {
                self.focus_in_rail = !self.focus_in_rail;
            }
            // up navigation
            b'k' | b'K' | b'w' | b'W' => {
                if self.focus_in_rail {
                    if self.rail_focus > 0 {
                        self.rail_focus -= 1;
                    }
                } else {
                    self.pane_focus_up();
                }
            }
            // down navigation
            b'j' | b'J' | b's' | b'S' => {
                if self.focus_in_rail {
                    if self.rail_focus < Route::ALL.len() - 1 {
                        self.rail_focus += 1;
                    }
                } else {
                    self.pane_focus_down();
                }
            }
            // Enter = navigate to rail selection or activate pane widget
            0x1C | b'\r' | b'\n' => {
                if self.focus_in_rail {
                    self.navigate(Route::from_index(self.rail_focus));
                } else {
                    self.pane_activate();
                }
            }
            // Escape = back to gateway or disarm
            0x01 | 0x1B => {
                if self.severe_arm == ArmState::Armed {
                    self.severe_arm = ArmState::Disarmed;
                    self.set_status("Disarmed", false);
                } else if self.route != Route::Gateway {
                    if self.has_pending_for(self.route) {
                        self.set_status("Unsaved changes — apply or revert first", true);
                    } else {
                        self.navigate(Route::Gateway);
                    }
                }
            }
            // focus rail
            b'h' | b'H' => {
                self.focus_in_rail = true;
            }
            // focus pane
            b'l' | b'L' => {
                self.focus_in_rail = false;
            }
            // 'a' key (apply staged) — scancode 0x1E
            0x1E | b'a' | b'A' => {
                if !self.focus_in_rail {
                    self.apply_pending();
                }
            }
            // 'r' key (revert) — scancode 0x13
            0x13 | b'r' | b'R' => {
                if !self.focus_in_rail {
                    self.revert_pending();
                }
            }
            // 'd' key (defaults)
            0x20 | b'd' | b'D' => {
                if !self.focus_in_rail {
                    self.restore_defaults();
                }
            }
            // forward to active chamber for chamber-specific keys
            _ => {
                self.chamber_key(key);
            }
        }
    }

    fn handle_click(&mut self, x: i32, y: i32) {
        self.frame_dirty = true;

        let rail_w = layout::RAIL_WIDTH;
        let strip_h = layout::STRIP_HEIGHT;
        let bar_h = layout::BAR_HEIGHT;

        // click in rail?
        if x < rail_w as i32 && y >= strip_h as i32 && y < (self.fb_h - bar_h) as i32 {
            let rel_y = (y - strip_h as i32) as u32;
            let item_h = layout::RAIL_ITEM_HEIGHT;
            let idx = (rel_y / item_h) as usize;
            if idx < Route::ALL.len() {
                self.rail_focus = idx;
                self.focus_in_rail = true;
                self.navigate(Route::from_index(idx));
            }
            return;
        }

        // click in command bar?
        if y >= (self.fb_h - bar_h) as i32 {
            self.handle_bar_click(x);
            return;
        }

        // click in pane — delegate to chamber
        if x >= rail_w as i32 && y >= strip_h as i32 {
            self.focus_in_rail = false;
            let pane_x = x - rail_w as i32;
            let pane_y = y - strip_h as i32;
            self.chamber_click(pane_x, pane_y);
        }
    }

    fn handle_bar_click(&mut self, x: i32) {
        let pane_w = self.fb_w - layout::RAIL_WIDTH;
        let btn_w = pane_w / 4;
        let base_x = layout::RAIL_WIDTH as i32;

        if x >= base_x && x < base_x + btn_w as i32 {
            self.apply_pending();
        } else if x >= base_x + btn_w as i32 && x < base_x + 2 * btn_w as i32 {
            self.revert_pending();
        } else if x >= base_x + 2 * btn_w as i32 && x < base_x + 3 * btn_w as i32 {
            self.restore_defaults();
        }
    }

    pub fn navigate(&mut self, target: Route) {
        if target == self.route {
            return;
        }
        if self.has_pending_for(self.route) {
            self.set_status("Unsaved changes — apply or revert first", true);
            return;
        }
        self.prev_route = self.route;
        self.route = target;
        self.pane_focus = 0;
        self.focus_in_rail = false;

        // refresh data for the target chamber
        match target {
            Route::NetObservatory => self.net_obs.refresh(),
            Route::SysObservatory => self.sys_obs.refresh(),
            Route::MistShore => self.mist.refresh(),
            _ => {}
        }
    }

    pub fn has_pending_for(&self, route: Route) -> bool {
        self.pending.iter().any(|p| p.chamber == route && p.state == FieldState::Edited)
    }

    pub fn has_any_pending(&self) -> bool {
        self.pending.iter().any(|p| p.state == FieldState::Edited)
    }

    fn apply_pending(&mut self) {
        let route = self.route;
        match route {
            Route::NetObservatory => crate::chambers::net_obs::apply(self),
            Route::MistShore => self.mist.apply(),
            Route::MirrorBasin => self.mirror.apply(),
            Route::HallOfMasks => crate::chambers::hall::apply(self),
            _ => {}
        }
        self.pending.retain(|p| p.chamber != route);
        self.set_status("Applied", false);
    }

    fn revert_pending(&mut self) {
        let route = self.route;
        match route {
            Route::NetObservatory => self.net_obs.revert(),
            Route::MistShore => self.mist.revert(),
            Route::MirrorBasin => self.mirror.revert(),
            _ => {}
        }
        self.pending.retain(|p| p.chamber != route);
        self.set_status("Reverted", false);
    }

    fn restore_defaults(&mut self) {
        let route = self.route;
        match route {
            Route::NetObservatory => self.net_obs.restore_defaults(),
            Route::MistShore => self.mist.restore_defaults(),
            Route::MirrorBasin => self.mirror.restore_defaults(),
            _ => {}
        }
        self.pending.retain(|p| p.chamber != route);
        self.set_status("Defaults restored", false);
    }

    fn pane_focus_up(&mut self) {
        self.clamp_pane_focus();
        if self.pane_focus > 0 {
            self.pane_focus -= 1;
        }
    }

    fn pane_focus_down(&mut self) {
        self.clamp_pane_focus();
        let max = self.pane_widget_count();
        if self.pane_focus + 1 < max {
            self.pane_focus += 1;
        }
    }

    fn pane_activate(&mut self) {
        self.clamp_pane_focus();
        let idx = self.pane_focus;
        match self.route {
            Route::Gateway => crate::chambers::gateway::activate(self, idx),
            Route::NetObservatory => crate::chambers::net_obs::activate(self, idx),
            Route::SysObservatory => crate::chambers::sys_obs::activate(self, idx),
            Route::MistShore => crate::chambers::mist::activate(self, idx),
            Route::MirrorBasin => crate::chambers::mirror::activate(self, idx),
            Route::Archive => self.archive.activate(idx),
            Route::HallOfMasks => crate::chambers::hall::activate(self, idx),
        }
    }

    fn chamber_key(&mut self, scancode: u8) {
        match self.route {
            Route::Gateway => crate::chambers::gateway::handle_key(self, scancode),
            Route::NetObservatory => crate::chambers::net_obs::handle_key(self, scancode),
            Route::SysObservatory => crate::chambers::sys_obs::handle_key(self, scancode),
            Route::MistShore => crate::chambers::mist::handle_key(self, scancode),
            Route::MirrorBasin => crate::chambers::mirror::handle_key(self, scancode),
            Route::Archive => self.archive.handle_key(scancode),
            Route::HallOfMasks => crate::chambers::hall::handle_key(self, scancode),
        }
    }

    fn chamber_click(&mut self, pane_x: i32, pane_y: i32) {
        let abs_x = pane_x + layout::RAIL_WIDTH as i32;
        let abs_y = pane_y + layout::STRIP_HEIGHT as i32;
        if let Some(idx) = self.hitbox_at(abs_x, abs_y) {
            self.pane_focus = idx;
            self.pane_activate();
            return;
        }

        // fallback for clicks outside registered controls: pick nearest slot.
        let count = self.pane_widget_count();
        if count > 0 {
            let pane_h = self
                .fb_h
                .saturating_sub(layout::STRIP_HEIGHT + layout::BAR_HEIGHT + 2 * layout::PANE_PAD)
                .max(1);
            let y = pane_y.max(0) as u32;
            let idx = ((y as u64 * count as u64) / pane_h as u64) as usize;
            self.pane_focus = idx.min(count - 1);
            self.pane_activate();
        }
    }

    fn clamp_pane_focus(&mut self) {
        let count = self.pane_widget_count();
        if count == 0 {
            self.pane_focus = 0;
        } else if self.pane_focus >= count {
            self.pane_focus = count - 1;
        }
    }

    fn pane_widget_count(&self) -> usize {
        match self.route {
            Route::Gateway => self.gateway.widget_count(),
            Route::MistShore => self.mist.widget_count(),
            Route::MirrorBasin => self.mirror.widget_count(),
            Route::NetObservatory => self.net_obs.widget_count(),
            Route::SysObservatory => self.sys_obs.widget_count(),
            Route::Archive => self.archive.widget_count(),
            Route::HallOfMasks => self.hall.widget_count(),
        }
    }

    pub fn set_status(&mut self, msg: &str, is_error: bool) {
        let n = msg.len().min(self.status_msg.len());
        self.status_msg[..n].copy_from_slice(&msg.as_bytes()[..n]);
        self.status_len = n;
        self.status_is_error = is_error;
        self.frame_dirty = true;
    }

    pub fn log_change(&mut self, chamber: Route, field: &'static str, desc: &str, destructive: bool) {
        let ts = libmorpheus::time::uptime_ms();
        let mut buf = [0u8; 128];
        let n = desc.len().min(128);
        buf[..n].copy_from_slice(&desc.as_bytes()[..n]);
        self.changelog.push(ChangeEntry {
            timestamp_ms: ts,
            chamber,
            field_name: field,
            description: buf,
            desc_len: n,
            destructive,
        });
    }

    pub fn mark_edited(&mut self, chamber: Route, field: &'static str) {
        // upsert
        if let Some(p) = self.pending.iter_mut().find(|p| p.chamber == chamber && p.field_name == field) {
            p.state = FieldState::Edited;
        } else {
            self.pending.push(PendingChange {
                chamber,
                field_name: field,
                state: FieldState::Edited,
            });
        }
    }

    fn render(&mut self) {
        layout::render_frame(self);
        self.rebuild_hitboxes();
    }

    fn clear_hitboxes(&mut self) {
        self.hitbox_count = 0;
    }

    fn push_hitbox(&mut self, x: i32, y: i32, w: i32, h: i32, widget_idx: usize) {
        if self.hitbox_count >= self.hitboxes.len() || w <= 0 || h <= 0 {
            return;
        }
        self.hitboxes[self.hitbox_count] = Hitbox {
            x,
            y,
            w,
            h,
            widget_idx,
        };
        self.hitbox_count += 1;
    }

    fn rebuild_hitboxes(&mut self) {
        self.clear_hitboxes();

        let count = self.pane_widget_count();
        if count == 0 {
            return;
        }

        let px = layout::RAIL_WIDTH as i32 + layout::PANE_PAD as i32;
        let pw = (self.fb_w.saturating_sub(layout::RAIL_WIDTH + 2 * layout::PANE_PAD)) as i32;
        if pw <= 0 {
            return;
        }

        match self.route {
            Route::Gateway => {
                let row_h = layout::row_step(self, 8) as i32;
                let base_y = layout::STRIP_HEIGHT as i32 + 2 * row_h;
                for i in 0..count {
                    let y = base_y + i as i32 * row_h;
                    self.push_hitbox(px, y, pw, row_h.max(14), i);
                }
            }
            Route::NetObservatory | Route::SysObservatory | Route::MistShore | Route::MirrorBasin => {
                let row_h = layout::row_step(self, 8) as i32;
                let base_y = layout::STRIP_HEIGHT as i32 + 40;
                for i in 0..count {
                    let y = base_y + i as i32 * row_h;
                    self.push_hitbox(px, y, pw, row_h.max(14), i);
                }
            }
            Route::HallOfMasks => {
                let row_h = (widgets::FONT_H + 12) as i32;
                let base_y = layout::STRIP_HEIGHT as i32 + 60;
                for i in 0..count {
                    let y = base_y + i as i32 * row_h;
                    self.push_hitbox(px, y, pw, row_h.max(14), i);
                }
            }
            Route::Archive => {
                // row picks in the log table (index 3 + visible row).
                let row_h = (widgets::FONT_H + 4) as i32;
                let table_y = layout::STRIP_HEIGHT as i32 + 60;
                let half = (pw / 2).max(20);
                self.push_hitbox(px, table_y - row_h, half, row_h, 0);
                self.push_hitbox(px + half, table_y - row_h, pw - half, row_h, 1);
                self.push_hitbox(px, table_y - 2 * row_h, pw, row_h, 2);

                let rows = count.saturating_sub(3);
                for i in 0..rows {
                    let y = table_y + i as i32 * row_h;
                    self.push_hitbox(px, y, pw, row_h.max(14), i + 3);
                }
            }
        }
    }

    fn hitbox_at(&self, x: i32, y: i32) -> Option<usize> {
        // last one wins in case of overlap
        for i in (0..self.hitbox_count).rev() {
            let hb = self.hitboxes[i];
            if x >= hb.x && y >= hb.y && x < hb.x + hb.w && y < hb.y + hb.h {
                return Some(hb.widget_idx);
            }
        }
        None
    }
}
