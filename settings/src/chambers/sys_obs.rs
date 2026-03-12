// system observatory — telemetry dashboard and power controls.
// memory, cpu uptime, heap stats, reboot, shutdown.
// the power buttons have arm→consequence→confirm because bricking on accident is not a feature.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{ArmState, Route, SettingsApp};
use crate::widgets;

use libmorpheus::sys;

const FIELD_REFRESH: usize = 0;
const FIELD_REBOOT: usize = 1;
const FIELD_SHUTDOWN: usize = 2;
const FIELD_FORCE_REBOOT: usize = 3;
const FIELD_FORCE_SHUTDOWN: usize = 4;
const FIELD_COUNT: usize = 5;

pub struct SysObsChamber {
    pub total_mem: u64,
    pub used_mem: u64,
    pub free_mem: u64,
    pub uptime_secs: u64,
    pub heap_used: u64,
    pub heap_total: u64,
    pub cpu_count: u32,
    pub idle_pct: u32,

    // power action arming
    pub reboot_arm: ArmState,
    pub shutdown_arm: ArmState,
}

impl SysObsChamber {
    pub fn new() -> Self {
        Self {
            total_mem: 0,
            used_mem: 0,
            free_mem: 0,
            uptime_secs: 0,
            heap_used: 0,
            heap_total: 0,
            cpu_count: 1,
            idle_pct: 0,
            reboot_arm: ArmState::Disarmed,
            shutdown_arm: ArmState::Disarmed,
        }
    }

    pub fn refresh(&mut self) {
        let mut info = sys::SysInfo::zeroed();
        let _ = sys::sysinfo(&mut info);
        self.total_mem = info.total_mem;
        self.used_mem = info.total_mem.saturating_sub(info.free_mem);
        self.free_mem = info.free_mem;
        self.uptime_secs = info.uptime_ms() / 1000;
        self.heap_used = info.heap_used;
        self.heap_total = info.heap_total;
        self.cpu_count = info.cpu_count;
        // idle fraction: idle_tsc / uptime_ticks. both are TSC-derived.
        self.idle_pct = if info.idle_tsc > 0 && info.uptime_ticks > 0 {
            ((info.idle_tsc * 100) / info.uptime_ticks) as u32
        } else {
            0
        };
    }

    pub fn widget_count(&self) -> usize {
        FIELD_COUNT
    }
}

pub fn activate(app: &mut SettingsApp, idx: usize) {
    match idx {
        FIELD_REFRESH => {
            app.sys_obs.refresh();
            app.set_status("Telemetry refreshed", false);
        }
        FIELD_REBOOT => {
            match app.sys_obs.reboot_arm {
                ArmState::Disarmed => {
                    app.sys_obs.reboot_arm = ArmState::Armed;
                    app.set_status("Reboot ARMED. Press again to confirm.", false);
                }
                ArmState::Armed => {
                    app.sys_obs.reboot_arm = ArmState::Confirmed;
                    app.log_change(Route::SysObservatory, "power", "Graceful reboot", true);
                    let _ = sys::reboot(false);
                }
                ArmState::Confirmed => {}
            }
        }
        FIELD_SHUTDOWN => {
            match app.sys_obs.shutdown_arm {
                ArmState::Disarmed => {
                    app.sys_obs.shutdown_arm = ArmState::Armed;
                    app.set_status("Shutdown ARMED. Press again to confirm.", false);
                }
                ArmState::Armed => {
                    app.sys_obs.shutdown_arm = ArmState::Confirmed;
                    app.log_change(Route::SysObservatory, "power", "Graceful shutdown", true);
                    let _ = sys::shutdown(false);
                }
                ArmState::Confirmed => {}
            }
        }
        FIELD_FORCE_REBOOT => {
            match app.sys_obs.reboot_arm {
                ArmState::Armed => {
                    app.log_change(Route::SysObservatory, "power", "Force reboot", true);
                    let _ = sys::reboot(true);
                }
                _ => {
                    app.set_status("Arm reboot first (Enter on Reboot)", false);
                }
            }
        }
        FIELD_FORCE_SHUTDOWN => {
            match app.sys_obs.shutdown_arm {
                ArmState::Armed => {
                    app.log_change(Route::SysObservatory, "power", "Force shutdown", true);
                    let _ = sys::shutdown(true);
                }
                _ => {
                    app.set_status("Arm shutdown first (Enter on Shutdown)", false);
                }
            }
        }
        _ => {}
    }
}

pub fn handle_key(app: &mut SettingsApp, scancode: u8) {
    if scancode == 0x01 {
        app.sys_obs.reboot_arm = ArmState::Disarmed;
        app.sys_obs.shutdown_arm = ArmState::Disarmed;
        app.set_status("Power actions disarmed", false);
    }
}

pub fn render(app: &SettingsApp) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;
    let sys = &app.sys_obs;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;
    let r2 = layout::row_step(app, 2);
    let r4 = layout::row_step(app, 4);
    let r8 = layout::row_step(app, 8);
    let r12 = layout::row_step(app, 12);

    // memory section
    layout::draw_section(app, px, cy, "Memory");
    cy += r4;

    let mut buf = [0u8; 32];
    let n = widgets::format_bytes(sys.total_mem, &mut buf);
    let total_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Total:", total_str, t.telemetry);
    cy += r2;

    let n = widgets::format_bytes(sys.used_mem, &mut buf);
    let used_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Used:", used_str, t.telemetry);
    cy += r2;

    let n = widgets::format_bytes(sys.free_mem, &mut buf);
    let free_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Free:", free_str, t.success);
    cy += r4;

    // memory usage bar
    let bar_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let pct = if sys.total_mem > 0 {
        ((sys.used_mem * 100) / sys.total_mem) as u32
    } else {
        0
    };
    let bar_color = if pct > 90 { t.destructive } else if pct > 70 { t.warning } else { t.signal };
    widgets::draw_bar(s, st, px, cy, bar_w, 10, pct, 100, bar_color, t.substrate, t.contour, w, h);
    cy += (r4 / 2).max(10);

    // heap section
    layout::draw_section(app, px, cy, "Heap");
    cy += r4;

    let n = widgets::format_bytes(sys.heap_used, &mut buf);
    let hu_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Used:", hu_str, t.telemetry);
    cy += r2;

    let n = widgets::format_bytes(sys.heap_total, &mut buf);
    let ht_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Total:", ht_str, t.telemetry);
    cy += r8;

    // cpu section
    layout::draw_section(app, px, cy, "CPU");
    cy += r4;

    let n = widgets::u64_to_str(sys.cpu_count as u64, &mut buf);
    let cpu_str = core::str::from_utf8(&buf[..n]).unwrap_or("1");
    layout::draw_kv(app, px, cy, "Cores:", cpu_str, t.telemetry);
    cy += r2;

    let n = widgets::u64_to_str(sys.idle_pct as u64, &mut buf);
    let idle_str = core::str::from_utf8(&buf[..n]).unwrap_or("0");
    layout::draw_kv(app, px, cy, "Idle %:", idle_str, t.telemetry);
    cy += r8;

    // uptime
    layout::draw_section(app, px, cy, "Uptime");
    cy += r4;

    let n = widgets::format_uptime(sys.uptime_secs, &mut buf);
    let up_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "Since boot:", up_str, t.immutable);
    cy += r12;

    // power controls
    layout::draw_section(app, px, cy, "Power Controls");
    cy += r4;

    layout::draw_button_row(app, px, cy, "Refresh Telemetry", FIELD_REFRESH, t.glyph);
    cy += r8;

    // reboot
    let rb_label = match sys.reboot_arm {
        ArmState::Disarmed => "Reboot (graceful)",
        ArmState::Armed => "!! CONFIRM REBOOT !!",
        ArmState::Confirmed => "(rebooting...)",
    };
    let rb_color = match sys.reboot_arm {
        ArmState::Disarmed => t.warning,
        ArmState::Armed => t.armed,
        ArmState::Confirmed => t.destructive,
    };
    layout::draw_button_row(app, px, cy, rb_label, FIELD_REBOOT, rb_color);
    cy += r4;

    if matches!(sys.reboot_arm, ArmState::Armed) {
        layout::draw_risk_band(app, px, cy, "System will restart. Unsaved work is lost.");
        cy += r4;
        layout::draw_button_row(app, px, cy, "Force Reboot (skip cleanup)", FIELD_FORCE_REBOOT, t.destructive);
        cy += r4;
    }

    // shutdown
    let sd_label = match sys.shutdown_arm {
        ArmState::Disarmed => "Shutdown (graceful)",
        ArmState::Armed => "!! CONFIRM SHUTDOWN !!",
        ArmState::Confirmed => "(shutting down...)",
    };
    let sd_color = match sys.shutdown_arm {
        ArmState::Disarmed => t.destructive,
        ArmState::Armed => t.armed,
        ArmState::Confirmed => t.destructive,
    };
    layout::draw_button_row(app, px, cy, sd_label, FIELD_SHUTDOWN, sd_color);
    cy += r4;

    if matches!(sys.shutdown_arm, ArmState::Armed) {
        layout::draw_risk_band(app, px, cy, "System will power off. No recovery without physical restart.");
        cy += r4;
        layout::draw_button_row(app, px, cy, "Force Shutdown (immediate)", FIELD_FORCE_SHUTDOWN, t.destructive);
    }
}
