pub mod focus;
pub mod input;
pub mod renderer;
pub mod surface_mgr;
pub mod vsync;

use crate::messages::*;
use channel::Channel;
use libmorpheus::compositor as compsys;

pub const MAX_WINDOWS: usize = 16;
/// Max bytes of the `de.win.state` blob compd publishes: `[focused_pid][n_min][min_pid…]` =
/// 8 + `MAX_WINDOWS` × 4. Sized so every window can be minimized at once.
pub const WIN_STATE_CAP: usize = 8 + MAX_WINDOWS * 4;
pub const TITLE_H: u32 = 22;
pub const BORDER: u32 = 1;
pub const CASCADE_STEP: i32 = 28;
/// Side of the square bottom-right resize grip. The hit-test region and the drawn handle share
/// this so the clickable area exactly matches the pixels (they drifted: 14 hit vs 12 drawn).
pub const GRIP: u32 = 14;

/// Text-cell geometry of a windowed client. Clients render an 8×16 bitmap-font cell grid
/// (the DE's `phosphor` font); windows are sized in whole cells and their content is blitted
/// 1:1, so a window's pixel size is always a multiple of these. compd reports a window's size
/// to its client in cells via the `CSI 8 ; rows ; cols t` resize report.
pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;

pub const TITLE_UNFOCUSED_RGB: (u8, u8, u8) = (40, 40, 40);
pub const TITLE_TEXT_RGB: (u8, u8, u8) = (255, 255, 255);
pub const BORDER_UNFOCUSED_RGB: (u8, u8, u8) = (85, 85, 85);
pub const CURSOR_RGB: (u8, u8, u8) = (255, 255, 255);

// Couples compd to shelld's layout — no ABI to query panel height.
pub const PANEL_H: u32 = 30;

pub struct ChildWindow {
    pub pid: u32,
    pub surface_ptr: *const u32,
    pub mapped: bool,
    pub surface_vaddr: u64,
    pub surface_pages: u64,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub src_stride: u32, // pixels (not bytes — fb stride is bytes; mismatch is deliberate).
    pub mouse_local_x: i32,
    pub mouse_local_y: i32,
    pub mouse_local_valid: bool,
    pub title: [u8; 64],
    pub title_len: usize,
    pub z_layer: u8, // 0=bg, 1=window, 3=overlay
    // Last content size (cols, rows) reported to the client via `CSI 8;rows;cols t`. 0 = never
    // sent; the client is notified once on map and again whenever a resize changes the cell count.
    pub sent_cols: u16,
    pub sent_rows: u16,
    // Floating geometry stashed when the window was maximized (Ctrl+Alt+5), so the next press
    // restores it. `Some` ⇒ the window is currently maximized; any other snap (or none) clears it.
    pub saved_rect: Option<(i32, i32, u32, u32)>,
    // Minimized (hidden): the window keeps its slot + geometry but is skipped by the renderer and
    // the hit-test, and is never focusable. Toggled by activating its taskbar chip (the shell's
    // focus request → `focus::consume_focus_request`). The z0 desktop is never minimized.
    pub minimized: bool,
}

#[derive(Clone, Copy)]
pub enum MouseCapture {
    Move {
        idx: usize,
        off_x: i32,
        off_y: i32,
    },
    Resize {
        idx: usize,
        start_mx: i32,
        start_my: i32,
        start_w: u32,
        start_h: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HitRegion {
    Content,
    Title,
    Close,
    Resize,
}

impl From<wm_geom::Region> for HitRegion {
    fn from(r: wm_geom::Region) -> Self {
        match r {
            wm_geom::Region::Title => HitRegion::Title,
            wm_geom::Region::Close => HitRegion::Close,
            wm_geom::Region::Resize => HitRegion::Resize,
            wm_geom::Region::Content => HitRegion::Content,
        }
    }
}

pub struct CompState {
    // renderer
    pub fb_ptr: *mut u32,
    pub fb_w: u32,
    pub fb_h: u32,
    pub fb_stride: u32, // pixels (fb_info.stride / 4).
    pub is_bgrx: bool,

    // surface_mgr
    pub windows: [Option<ChildWindow>; MAX_WINDOWS],
    pub cascade_n: i32,
    pub surface_buf: [compsys::SurfaceEntry; MAX_WINDOWS],
    pub desktop_idx: Option<usize>, // shelld's z0 surface slot.

    // focus
    pub focused: Option<usize>,
    // Last `de.focus.req` token serviced (baselined at startup so a stale cross-boot request can't
    // activate a window before the desktop is up). compd acts only on a strictly-different token.
    pub focus_req_token: u32,
    // Last window-state blob (`de.win.state`) published to the shell, cached so the per-frame
    // publish only writes (and fsyncs) the persist key when the focus/minimized snapshot changes.
    pub win_state_buf: [u8; WIN_STATE_CAP],
    pub win_state_len: usize,

    // input
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub last_buttons: u8,
    pub capture: Option<MouseCapture>,
    // While a title-bar drag is over a screen-edge snap trigger, the zone the window will tile to on
    // release (Aero Snap). The renderer highlights it as a translucent preview; `route_mouse_spatial`
    // applies it on release. `None` whenever the pointer is away from any edge or no Move drag is live.
    pub snap_preview: Option<wm_geom::SnapZone>,
    // Last title-bar press as `(uptime_ms, window_idx)`, for double-click detection: a second press
    // on the same window within DOUBLE_CLICK_MS maximizes/restores it. Consumed (→ `None`) on a
    // double so a rapid third press starts a fresh pair; otherwise overwritten by each new press.
    pub last_title_press: Option<(u64, usize)>,

    // Keyboard decoder: PS/2 Set 1 scancodes → terminal bytes, carrying the modifier state
    // machine across reads (release-edge decode, resilient to this kernel's make corruption).
    pub kbd: keymap::ScanDecoder,
    // active keyboard layout (hot-swappable from /system/keymap.kmap).
    pub keymap: keymap::Keymap,

    pub desktop_rgb: (u8, u8, u8),
    pub title_focused_rgb: (u8, u8, u8),
    pub border_focused_rgb: (u8, u8, u8),

    // SPSC island channels.
    pub ch_input_to_focus: Channel<InputMsg, 16>,
    pub ch_mouse_spatial: Channel<MouseSpatialMsg, 32>,
    pub ch_mouse_route: Channel<MouseZRouteMsg, 32>,
}

impl CompState {
    pub fn new(fb_ptr: *mut u32, fb_w: u32, fb_h: u32, fb_stride_px: u32, is_bgrx: bool) -> Self {
        const NONE: Option<ChildWindow> = None;
        Self {
            fb_ptr,
            fb_w,
            fb_h,
            fb_stride: fb_stride_px,
            is_bgrx,
            windows: [NONE; MAX_WINDOWS],
            cascade_n: 0,
            surface_buf: [zeroed_surface_entry(); MAX_WINDOWS],
            desktop_idx: None,
            focused: None,
            focus_req_token: 0,
            win_state_buf: [0u8; WIN_STATE_CAP],
            win_state_len: 0,
            mouse_x: (fb_w / 2) as i32,
            mouse_y: (fb_h / 2) as i32,
            last_buttons: 0,
            capture: None,
            snap_preview: None,
            last_title_press: None,
            kbd: keymap::ScanDecoder::new(),
            keymap: keymap::german_default(),
            desktop_rgb: (26, 26, 46),
            title_focused_rgb: (0, 85, 0),
            border_focused_rgb: (0, 170, 0),
            ch_input_to_focus: Channel::new(),
            ch_mouse_spatial: Channel::new(),
            ch_mouse_route: Channel::new(),
        }
    }

    /// Pack RGB into framebuffer pixel; BGRX byte order on most UEFI GOPs.
    #[inline(always)]
    pub fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
    }

    pub fn apply_desktop_appearance(&mut self, a: &libmorpheus::desktop::DesktopAppearance) {
        self.desktop_rgb = a.desktop_rgb;
        self.title_focused_rgb = a.title_focus_rgb;
        self.border_focused_rgb = a.border_focus_rgb;
    }
}

pub const fn zeroed_surface_entry() -> compsys::SurfaceEntry {
    compsys::SurfaceEntry {
        pid: 0,
        _pad: 0,
        phys_addr: 0,
        pages: 0,
        width: 0,
        height: 0,
        stride: 0,
        format: 0,
        dirty: 0,
        _pad2: 0,
    }
}
