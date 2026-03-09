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

pub const TITLE_FOCUSED_RGB: (u8, u8, u8) = (0, 85, 0);
pub const TITLE_UNFOCUSED_RGB: (u8, u8, u8) = (40, 40, 40);
pub const TITLE_TEXT_RGB: (u8, u8, u8) = (255, 255, 255);
pub const BORDER_FOCUSED_RGB: (u8, u8, u8) = (0, 170, 0);
pub const BORDER_UNFOCUSED_RGB: (u8, u8, u8) = (85, 85, 85);
pub const DESKTOP_RGB: (u8, u8, u8) = (26, 26, 46);
pub const CURSOR_RGB: (u8, u8, u8) = (255, 255, 255);

// shelld's panel height. hardcoded here so compd can re-blit it above windows.
// yes it couples us to shelld's layout. no there's no ABI for "tell me your panel height".
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
    pub src_stride: u32, // in pixels. not bytes. the framebuffer stride IS bytes. yes again.
    pub title: [u8; 64],
    pub title_len: usize,
    pub z_layer: u8, // 0=desktop background, 1=normal window, 3=overlay
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
    // --- renderer island owns these ---
    pub fb_ptr: *mut u32,
    pub fb_w: u32,
    pub fb_h: u32,
    pub fb_stride: u32, // stride in PIXELS (fb_info.stride / 4). yes confusing. no we can't change it.
    pub is_bgrx: bool,

    // --- surface_mgr island owns these ---
    pub windows: [Option<ChildWindow>; MAX_WINDOWS],
    pub cascade_n: i32,
    pub surface_buf: [compsys::SurfaceEntry; MAX_WINDOWS],
    pub desktop_idx: Option<usize>, // slot index of shelld's desktop surface (z_layer 0)

    // --- focus island owns these ---
    pub focused: Option<usize>,

    // --- input island owns these ---
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub last_buttons: u8,
    pub capture: Option<MouseCapture>,

    // --- channels (SPSC) ---
    pub ch_input_to_focus: Channel<InputMsg, 16>,
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
            ch_input_to_focus: Channel::new(),
        }
    }

    // pack the pixel. bgrx not rgbx. the b comes first because uefi said so and uefi answers to no one.
    #[inline(always)]
    pub fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
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
