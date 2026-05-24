pub mod focus;
pub mod input;
pub mod renderer;
pub mod surface_mgr;
pub mod vsync;

use crate::messages::*;
use channel::Channel;
use libmorpheus::compositor as compsys;

pub const MAX_WINDOWS: usize = 16;
pub const TITLE_H: u32 = 22;
pub const BORDER: u32 = 1;
pub const CASCADE_STEP: i32 = 28;

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

    // input
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub last_buttons: u8,
    pub capture: Option<MouseCapture>,

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
            mouse_x: (fb_w / 2) as i32,
            mouse_y: (fb_h / 2) as i32,
            last_buttons: 0,
            capture: None,
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
