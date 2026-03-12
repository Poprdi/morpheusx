// network observatory — operational connectivity controls.
// DHCP/static toggle, hostname, DNS, link status, MAC, stats.
// the most syscall-heavy chamber. exercises SYS_NET_CFG and SYS_NIC_INFO.

use crate::layout::{self, PANE_PAD, RAIL_WIDTH, STRIP_HEIGHT};
use crate::state::{Route, SettingsApp};
use crate::widgets;

use libmorpheus::net;

// editable field indices
const FIELD_DHCP_TOGGLE: usize = 0;
const FIELD_HOSTNAME: usize = 1;
const FIELD_IP: usize = 2;
const FIELD_PREFIX: usize = 3;
const FIELD_GATEWAY: usize = 4;
const FIELD_DNS1: usize = 5;
const FIELD_DNS2: usize = 6;
const FIELD_APPLY: usize = 7;
const FIELD_ACTIVATE: usize = 8;
const FIELD_REFRESH: usize = 9;
const FIELD_COUNT: usize = 10;

pub struct NetObsChamber {
    // live state from kernel
    pub state: u32,
    pub flags: u32,
    pub ip: u32,
    pub prefix_len: u8,
    pub gateway: u32,
    pub dns1: u32,
    pub dns2: u32,
    pub mac: [u8; 6],
    pub hostname: [u8; 64],
    pub hostname_len: usize,
    pub link_up: bool,
    pub mtu: u32,

    // stats
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,

    // edited fields (for pending change tracking)
    pub edit_dhcp: bool,
    pub edit_hostname: [u8; 64],
    pub edit_hostname_len: usize,
    pub edit_ip: [u8; 16],
    pub edit_ip_len: usize,
    pub edit_prefix: u8,
    pub edit_gateway: [u8; 16],
    pub edit_gw_len: usize,
    pub edit_dns1: [u8; 16],
    pub edit_dns1_len: usize,
    pub edit_dns2: [u8; 16],
    pub edit_dns2_len: usize,

    // which text field is being edited (cursor active)
    pub editing_field: Option<usize>,
}

impl NetObsChamber {
    pub fn new() -> Self {
        Self {
            state: 0,
            flags: 0,
            ip: 0,
            prefix_len: 0,
            gateway: 0,
            dns1: 0,
            dns2: 0,
            mac: [0; 6],
            hostname: [0; 64],
            hostname_len: 0,
            link_up: false,
            mtu: 0,
            tx_packets: 0,
            rx_packets: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            edit_dhcp: true,
            edit_hostname: [0; 64],
            edit_hostname_len: 0,
            edit_ip: [0; 16],
            edit_ip_len: 0,
            edit_prefix: 24,
            edit_gateway: [0; 16],
            edit_gw_len: 0,
            edit_dns1: [0; 16],
            edit_dns1_len: 0,
            edit_dns2: [0; 16],
            edit_dns2_len: 0,
            editing_field: None,
        }
    }

    pub fn refresh(&mut self) {
        if let Ok(cfg) = net::net_config() {
            self.state = cfg.state;
            self.flags = cfg.flags;
            self.ip = cfg.ipv4_addr;
            self.prefix_len = cfg.prefix_len;
            self.gateway = cfg.gateway;
            self.dns1 = cfg.dns_primary;
            self.dns2 = cfg.dns_secondary;
            self.mac = [cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]];
            self.mtu = cfg.mtu;

            // hostname
            let hlen = cfg.hostname.iter().position(|&b| b == 0).unwrap_or(cfg.hostname.len());
            self.hostname[..hlen].copy_from_slice(&cfg.hostname[..hlen]);
            self.hostname_len = hlen;

            self.edit_dhcp = (cfg.flags & net::NET_FLAG_DHCP) != 0;

            // populate edit fields from live state
            self.sync_edit_from_live();
        }

        if let Ok(info) = net::nic_info() {
            self.link_up = info.link_up != 0;
        }

        if let Ok(stats) = net::net_stats() {
            self.tx_packets = stats.tx_packets;
            self.rx_packets = stats.rx_packets;
            self.tx_bytes = stats.tx_bytes;
            self.rx_bytes = stats.rx_bytes;
        }
    }

    fn sync_edit_from_live(&mut self) {
        // hostname
        self.edit_hostname[..self.hostname_len].copy_from_slice(&self.hostname[..self.hostname_len]);
        self.edit_hostname_len = self.hostname_len;

        // ip
        self.edit_ip_len = widgets::format_ip(self.ip, &mut self.edit_ip);
        self.edit_prefix = self.prefix_len;
        self.edit_gw_len = widgets::format_ip(self.gateway, &mut self.edit_gateway);
        self.edit_dns1_len = widgets::format_ip(self.dns1, &mut self.edit_dns1);
        self.edit_dns2_len = widgets::format_ip(self.dns2, &mut self.edit_dns2);
    }

    pub fn widget_count(&self) -> usize {
        FIELD_COUNT
    }

    pub fn revert(&mut self) {
        self.editing_field = None;
        self.sync_edit_from_live();
    }

    pub fn restore_defaults(&mut self) {
        self.edit_dhcp = true;
        self.edit_hostname_len = 0;
        self.editing_field = None;
    }

    fn text_insert(&mut self, field: usize, ch: u8) {
        let (buf, len) = self.field_buf_mut(field);
        if *len < buf.len() {
            buf[*len] = ch;
            *len += 1;
        }
    }

    fn text_backspace(&mut self, field: usize) {
        let (_, len) = self.field_buf_mut(field);
        if *len > 0 {
            *len -= 1;
        }
    }

    fn field_buf_mut(&mut self, field: usize) -> (&mut [u8], &mut usize) {
        match field {
            FIELD_HOSTNAME => (&mut self.edit_hostname, &mut self.edit_hostname_len),
            FIELD_IP => (&mut self.edit_ip, &mut self.edit_ip_len),
            FIELD_GATEWAY => (&mut self.edit_gateway, &mut self.edit_gw_len),
            FIELD_DNS1 => (&mut self.edit_dns1, &mut self.edit_dns1_len),
            FIELD_DNS2 => (&mut self.edit_dns2, &mut self.edit_dns2_len),
            _ => (&mut self.edit_hostname, &mut self.edit_hostname_len),
        }
    }
}

pub fn activate(app: &mut SettingsApp, idx: usize) {
    match idx {
        FIELD_DHCP_TOGGLE => {
            app.net_obs.edit_dhcp = !app.net_obs.edit_dhcp;
            app.mark_edited(Route::NetObservatory, "dhcp");
        }
        FIELD_HOSTNAME | FIELD_IP | FIELD_GATEWAY | FIELD_DNS1 | FIELD_DNS2 => {
            app.net_obs.editing_field = Some(idx);
        }
        FIELD_PREFIX => {
            app.net_obs.edit_prefix = match app.net_obs.edit_prefix {
                8 => 16,
                16 => 24,
                24 => 32,
                _ => 8,
            };
            app.mark_edited(Route::NetObservatory, "prefix");
        }
        FIELD_APPLY => {
            if apply(app) {
                app.clear_pending_for(Route::NetObservatory);
                app.net_obs.editing_field = None;
            }
        }
        FIELD_ACTIVATE => match net::net_activate() {
            Ok(rc) => {
                app.net_obs.refresh();
                if rc == 0 {
                    app.set_status("Networking activated", false);
                } else {
                    app.set_status("Networking already active", false);
                }
            }
            Err(_) => {
                app.set_status("Networking activation failed", true);
            }
        },
        FIELD_REFRESH => {
            app.net_obs.refresh();
            app.set_status("Network refreshed", false);
        }
        _ => {}
    }
}

pub fn apply(app: &mut SettingsApp) -> bool {
    // hostname — copy to local buf so we don't hold a borrow on app across log_change
    let hl = app.net_obs.edit_hostname_len;
    if hl > 0 {
        let mut hn_buf = [0u8; 64];
        hn_buf[..hl].copy_from_slice(&app.net_obs.edit_hostname[..hl]);
        let hn = core::str::from_utf8(&hn_buf[..hl]).unwrap_or("");
        if let Err(_) = net::net_set_hostname(hn) {
            app.set_status("Hostname set failed", true);
            return false;
        }
        app.log_change(Route::NetObservatory, "hostname", hn, false);
    }

    let dhcp = app.net_obs.edit_dhcp;
    if dhcp {
        if let Err(_) = net::net_dhcp() {
            app.set_status("DHCP request failed", true);
            return false;
        }
        app.log_change(Route::NetObservatory, "mode", "Switched to DHCP", false);
    } else {
        let ip_len = app.net_obs.edit_ip_len;
        let gw_len = app.net_obs.edit_gw_len;
        let prefix = app.net_obs.edit_prefix;
        if prefix == 0 || prefix > 32 {
            app.set_status("Invalid prefix length", true);
            return false;
        }

        let ip = match parse_ipv4_strict(&app.net_obs.edit_ip[..ip_len]) {
            Ok(v) => v,
            Err(_) => {
                app.set_status("Invalid static IP", true);
                return false;
            }
        };
        let gw = match parse_ipv4_strict(&app.net_obs.edit_gateway[..gw_len]) {
            Ok(v) => v,
            Err(_) => {
                app.set_status("Invalid gateway IP", true);
                return false;
            }
        };
        if let Err(_) = net::net_static_ip(ip, prefix, gw) {
            app.set_status("Static IP set failed", true);
            return false;
        }
        app.log_change(Route::NetObservatory, "mode", "Switched to static", false);

        let d1_len = app.net_obs.edit_dns1_len;
        let d2_len = app.net_obs.edit_dns2_len;
        let d1 = match parse_ipv4_or_empty(&app.net_obs.edit_dns1[..d1_len]) {
            Ok(v) => v,
            Err(_) => {
                app.set_status("Invalid primary DNS", true);
                return false;
            }
        };
        let d2 = match parse_ipv4_or_empty(&app.net_obs.edit_dns2[..d2_len]) {
            Ok(v) => v,
            Err(_) => {
                app.set_status("Invalid secondary DNS", true);
                return false;
            }
        };
        let servers = [d1, d2];
        if let Err(_) = net::dns_set_servers(&servers) {
            app.set_status("DNS set failed", true);
            return false;
        }
    }

    app.net_obs.refresh();
    app.set_status("Network config applied", false);
    true
}

pub fn handle_key(app: &mut SettingsApp, scancode: u8) {
    if let Some(field) = app.net_obs.editing_field {
        match scancode {
            0x01 => {
                app.net_obs.editing_field = None;
            }
            0x1C => {
                app.net_obs.editing_field = None;
                app.mark_edited(Route::NetObservatory, field_name(field));
            }
            0x0E => {
                app.net_obs.text_backspace(field);
                app.mark_edited(Route::NetObservatory, field_name(field));
            }
            _ => {
                if let Some(ch) = scancode_to_char(scancode) {
                    app.net_obs.text_insert(field, ch);
                    app.mark_edited(Route::NetObservatory, field_name(field));
                }
            }
        }
        app.frame_dirty = true;
    }
}

pub fn render(app: &SettingsApp) {
    let t = &app.theme;
    let net = &app.net_obs;

    let px = RAIL_WIDTH + PANE_PAD;
    let mut cy = STRIP_HEIGHT + PANE_PAD;
    let r2 = layout::row_step(app, 2);
    let r4 = layout::row_step(app, 4);
    let r8 = layout::row_step(app, 8);
    let r12 = layout::row_step(app, 12);

    // link status section
    layout::draw_section(app, px, cy, "Link Status");
    cy += r4;

    let link_str = if net.link_up { "UP" } else { "DOWN" };
    let link_color = if net.link_up { t.success } else { t.destructive };
    layout::draw_kv(app, px, cy, "Link:", link_str, link_color);
    cy += r2;

    let mut mac_buf = [0u8; 17];
    let mac_len = widgets::format_mac(&net.mac, &mut mac_buf);
    let mac_str = core::str::from_utf8(&mac_buf[..mac_len]).unwrap_or("??");
    layout::draw_kv(app, px, cy, "MAC:", mac_str, t.immutable);
    cy += r2;

    let mut mtu_buf = [0u8; 8];
    let mtu_len = widgets::u64_to_str(net.mtu as u64, &mut mtu_buf);
    let mtu_str = core::str::from_utf8(&mtu_buf[..mtu_len]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "MTU:", mtu_str, t.telemetry);
    cy += r8;

    // state
    let state_str = match net.state {
        0 => "Unconfigured",
        1 => "DHCP Discovering",
        2 => "Ready",
        3 => "Error",
        _ => "Unknown",
    };
    let state_color = match net.state {
        2 => t.success,
        3 => t.destructive,
        _ => t.warning,
    };
    layout::draw_kv(app, px, cy, "State:", state_str, state_color);
    cy += r8;

    // configuration section
    layout::draw_section(app, px, cy, "Configuration");
    cy += r4;

    // DHCP toggle
    let dhcp_label = if net.edit_dhcp { "[X] DHCP" } else { "[ ] Static" };
    layout::draw_button_row(app, px, cy, dhcp_label, FIELD_DHCP_TOGGLE, t.glyph);
    cy += r8;

    // hostname
    let hn = core::str::from_utf8(&net.edit_hostname[..net.edit_hostname_len]).unwrap_or("");
    let hn_display = if hn.is_empty() { "(none)" } else { hn };
    let hn_editing = net.editing_field == Some(FIELD_HOSTNAME);
    draw_editable_field(app, px, cy, "Hostname:", hn_display, FIELD_HOSTNAME, hn_editing);
    cy += r8;

    if !net.edit_dhcp {
        // static ip fields
        let ip_str = core::str::from_utf8(&net.edit_ip[..net.edit_ip_len]).unwrap_or("0.0.0.0");
        let ip_editing = net.editing_field == Some(FIELD_IP);
        draw_editable_field(app, px, cy, "IP Address:", ip_str, FIELD_IP, ip_editing);
        cy += r8;

        let mut pfx_buf = [0u8; 4];
        let pfx_len = widgets::u64_to_str(net.edit_prefix as u64, &mut pfx_buf);
        let pfx_str = core::str::from_utf8(&pfx_buf[..pfx_len]).unwrap_or("24");
        layout::draw_field_row(app, px, cy, "Prefix Len:", pfx_str, false, FIELD_PREFIX);
        cy += r8;

        let gw_str = core::str::from_utf8(&net.edit_gateway[..net.edit_gw_len]).unwrap_or("0.0.0.0");
        let gw_editing = net.editing_field == Some(FIELD_GATEWAY);
        draw_editable_field(app, px, cy, "Gateway:", gw_str, FIELD_GATEWAY, gw_editing);
        cy += r8;

        let d1_str = core::str::from_utf8(&net.edit_dns1[..net.edit_dns1_len]).unwrap_or("0.0.0.0");
        let d1_editing = net.editing_field == Some(FIELD_DNS1);
        draw_editable_field(app, px, cy, "DNS Primary:", d1_str, FIELD_DNS1, d1_editing);
        cy += r8;

        let d2_str = core::str::from_utf8(&net.edit_dns2[..net.edit_dns2_len]).unwrap_or("0.0.0.0");
        let d2_editing = net.editing_field == Some(FIELD_DNS2);
        draw_editable_field(app, px, cy, "DNS Secondary:", d2_str, FIELD_DNS2, d2_editing);
        cy += r8;
    } else {
        // show current DHCP-assigned values as read-only
        let mut ip_buf = [0u8; 16];
        let ip_len = widgets::format_ip(net.ip, &mut ip_buf);
        let ip_str = core::str::from_utf8(&ip_buf[..ip_len]).unwrap_or("0.0.0.0");
        layout::draw_kv(app, px, cy, "Assigned IP:", ip_str, t.immutable);
        cy += r4;

        let mut gw_buf = [0u8; 16];
        let gw_len = widgets::format_ip(net.gateway, &mut gw_buf);
        let gw_str = core::str::from_utf8(&gw_buf[..gw_len]).unwrap_or("0.0.0.0");
        layout::draw_kv(app, px, cy, "Gateway:", gw_str, t.immutable);
        cy += r4;

        let mut d1_buf = [0u8; 16];
        let d1_len = widgets::format_ip(net.dns1, &mut d1_buf);
        let d1_str = core::str::from_utf8(&d1_buf[..d1_len]).unwrap_or("0.0.0.0");
        layout::draw_kv(app, px, cy, "DNS:", d1_str, t.immutable);
        cy += r8;
    }

    // action buttons
    layout::draw_button_row(app, px, cy, "Apply Network Config", FIELD_APPLY, t.signal);
    cy += r8;
    layout::draw_button_row(app, px, cy, "Activate Networking", FIELD_ACTIVATE, t.warning);
    cy += r8;
    layout::draw_button_row(app, px, cy, "Refresh", FIELD_REFRESH, t.glyph);
    cy += r12;

    // traffic stats section
    layout::draw_section(app, px, cy, "Traffic");
    cy += r4;

    let mut buf = [0u8; 32];
    let n = widgets::u64_to_str(net.tx_packets, &mut buf);
    let tx_p = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "TX Packets:", tx_p, t.telemetry);
    cy += r2;

    let n = widgets::u64_to_str(net.rx_packets, &mut buf);
    let rx_p = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "RX Packets:", rx_p, t.telemetry);
    cy += r2;

    let n = widgets::format_bytes(net.tx_bytes, &mut buf);
    let tx_b = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "TX Bytes:", tx_b, t.telemetry);
    cy += r2;

    let n = widgets::format_bytes(net.rx_bytes, &mut buf);
    let rx_b = core::str::from_utf8(&buf[..n]).unwrap_or("?");
    layout::draw_kv(app, px, cy, "RX Bytes:", rx_b, t.telemetry);
}

fn draw_editable_field(app: &SettingsApp, x: u32, y: u32, label: &str, value: &str, field_idx: usize, editing: bool) {
    let s = app.surface;
    let st = app.fb_stride;
    let w = app.fb_w;
    let h = app.fb_h;
    let t = &app.theme;

    let is_focused = !app.focus_in_rail && app.pane_focus == field_idx;
    let bg = if editing { t.input_bg } else if is_focused { t.surface } else { t.substrate };
    let row_w = (w - RAIL_WIDTH).saturating_sub(2 * PANE_PAD);
    let row_h = layout::row_step(app, 4);
    app.register_widget_hitbox(x, y, row_w, row_h, field_idx);
    widgets::fill_rect(s, st, x, y, row_w, row_h, bg, w, h);

    let border_color = if editing {
        t.signal
    } else if is_focused {
        t.focus_ring
    } else {
        t.contour
    };
    widgets::rect_outline(s, st, x, y, row_w, row_h, border_color, w, h);

    let ty = y + row_h.saturating_sub(widgets::FONT_H) / 2;
    let label_w = (row_w / 3).clamp(12 * widgets::FONT_W, 22 * widgets::FONT_W);
    let label_chars = label_w.saturating_sub(8) / widgets::FONT_W;
    widgets::draw_str_trunc(s, st, x + 4, ty, label, t.glyph_dim, bg, w, h, label_chars as usize);
    let vx = x + label_w;
    let value_chars = row_w.saturating_sub(label_w + 6) / widgets::FONT_W;
    widgets::draw_str_trunc(s, st, vx, ty, value, t.glyph, bg, w, h, value_chars as usize);

    // cursor
    if editing {
        let cursor_x = vx + value.len() as u32 * widgets::FONT_W;
        widgets::fill_rect(s, st, cursor_x, ty, 2, widgets::FONT_H, t.focus_ring, w, h);
    }
}

fn field_name(idx: usize) -> &'static str {
    match idx {
        FIELD_DHCP_TOGGLE => "dhcp",
        FIELD_HOSTNAME => "hostname",
        FIELD_IP => "ip",
        FIELD_PREFIX => "prefix",
        FIELD_GATEWAY => "gateway",
        FIELD_DNS1 => "dns1",
        FIELD_DNS2 => "dns2",
        _ => "unknown",
    }
}

fn parse_ipv4_or_empty(buf: &[u8]) -> Result<u32, ()> {
    if buf.is_empty() {
        return Ok(0);
    }
    parse_ipv4_strict(buf)
}

fn parse_ipv4_strict(buf: &[u8]) -> Result<u32, ()> {
    let mut octets = [0u8; 4];
    let mut oi = 0usize;
    let mut acc: u32 = 0;
    let mut saw_digit = false;

    for &b in buf {
        if b == b'.' {
            if !saw_digit || oi >= 3 || acc > 255 {
                return Err(());
            }
            octets[oi] = acc as u8;
            oi += 1;
            acc = 0;
            saw_digit = false;
            continue;
        }

        if !b.is_ascii_digit() {
            return Err(());
        }

        saw_digit = true;
        acc = acc.saturating_mul(10).saturating_add((b - b'0') as u32);
        if acc > 255 {
            return Err(());
        }
    }

    if !saw_digit || oi != 3 || acc > 255 {
        return Err(());
    }
    octets[3] = acc as u8;
    Ok(u32::from_be_bytes(octets))
}

pub fn scancode_to_char(sc: u8) -> Option<u8> {
    if sc.is_ascii_graphic() || sc == b' ' {
        return Some(sc.to_ascii_lowercase());
    }
    match sc {
        0x02..=0x0A => Some(b'1' + (sc - 0x02)),
        0x0B => Some(b'0'),
        0x10 => Some(b'q'),
        0x11 => Some(b'w'),
        0x12 => Some(b'e'),
        0x13 => Some(b'r'),
        0x14 => Some(b't'),
        0x15 => Some(b'y'),
        0x16 => Some(b'u'),
        0x17 => Some(b'i'),
        0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x1E => Some(b'a'),
        0x1F => Some(b's'),
        0x20 => Some(b'd'),
        0x21 => Some(b'f'),
        0x22 => Some(b'g'),
        0x23 => Some(b'h'),
        0x24 => Some(b'j'),
        0x25 => Some(b'k'),
        0x26 => Some(b'l'),
        0x2C => Some(b'z'),
        0x2D => Some(b'x'),
        0x2E => Some(b'c'),
        0x2F => Some(b'v'),
        0x30 => Some(b'b'),
        0x31 => Some(b'n'),
        0x32 => Some(b'm'),
        0x33 => Some(b','),
        0x34 => Some(b'.'),
        0x27 => Some(b';'),
        0x28 => Some(b'\''),
        0x0C => Some(b'-'),
        _ => None,
    }
}
