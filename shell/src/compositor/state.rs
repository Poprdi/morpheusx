use super::*;

pub(super) const MAX_WINDOWS: usize = 16;
pub(super) const TITLE_H: u32 = 22;
pub(super) const BORDER: u32 = 1;
pub(super) const CASCADE_STEP: i32 = 28;

pub(super) const TITLE_FOCUSED_RGB: (u8, u8, u8) = (0, 85, 0);
pub(super) const TITLE_UNFOCUSED_RGB: (u8, u8, u8) = (40, 40, 40);
pub(super) const TITLE_TEXT_RGB: (u8, u8, u8) = (255, 255, 255);
pub(super) const BORDER_FOCUSED_RGB: (u8, u8, u8) = (0, 170, 0);
pub(super) const BORDER_UNFOCUSED_RGB: (u8, u8, u8) = (85, 85, 85);
pub(super) const DESKTOP_RGB: (u8, u8, u8) = (26, 26, 46);
pub(super) const CURSOR_RGB: (u8, u8, u8) = (255, 255, 255);

pub(super) struct ChildWindow {
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
    pub src_stride: u32,
    pub title: [u8; 64],
    pub title_len: usize,
}

#[derive(Clone, Copy)]
pub(super) enum MouseCapture {
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
pub(super) enum HitRegion {
    Content,
    Title,
    Close,
    Resize,
}

pub struct Compositor {
    pub(super) windows: [Option<ChildWindow>; MAX_WINDOWS],
    pub(super) focused: Option<usize>,
    pub(super) fb_w: u32,
    pub(super) fb_h: u32,
    pub(super) fb_stride: u32,
    pub(super) is_bgrx: bool,
    pub(super) cascade_n: i32,
    pub(super) mouse_x: i32,
    pub(super) mouse_y: i32,
    pub(super) last_buttons: u8,
    pub(super) capture: Option<MouseCapture>,
    pub did_compose: bool,
    pub(super) surface_buf: [compsys::SurfaceEntry; MAX_WINDOWS],
}

impl Compositor {
    pub fn new(fb: &Framebuffer) -> Self {
        const NONE: Option<ChildWindow> = None;
        Self {
            windows: [NONE; MAX_WINDOWS],
            focused: None,
            fb_w: fb.width,
            fb_h: fb.height,
            fb_stride: fb.stride_px(),
            is_bgrx: fb.is_bgrx(),
            cascade_n: 0,
            mouse_x: (fb.width / 2) as i32,
            mouse_y: (fb.height / 2) as i32,
            last_buttons: 0,
            capture: None,
            did_compose: false,
            surface_buf: [zeroed_surface_entry(); MAX_WINDOWS],
        }
    }

    pub fn add_child(&mut self, pid: u32, name: &str) {
        for (i, slot) in self.windows.iter_mut().enumerate() {
            if slot.is_none() {
                let mut title = [0u8; 64];
                let len = name.len().min(63);
                title[..len].copy_from_slice(&name.as_bytes()[..len]);

                let max_w = self.fb_w.saturating_sub(40);
                let max_h = self.fb_h.saturating_sub(TITLE_H + 40);
                let w = ((self.fb_w as u64 * 58) / 100) as u32;
                let h = ((self.fb_h as u64 * 58) / 100) as u32;
                let w = w.clamp(320, max_w.max(320));
                let h = h.clamp(220, max_h.max(220));

                let step = CASCADE_STEP * (self.cascade_n % 5);
                let x = (20 + step).clamp(0, (self.fb_w as i32 - w as i32).max(0));
                let y = (TITLE_H as i32 + 20 + step).clamp(
                    TITLE_H as i32,
                    (self.fb_h as i32 - h as i32).max(TITLE_H as i32),
                );

                *slot = Some(ChildWindow {
                    pid,
                    surface_ptr: core::ptr::null(),
                    mapped: false,
                    surface_vaddr: 0,
                    surface_pages: 0,
                    x,
                    y,
                    w,
                    h,
                    src_w: self.fb_w,
                    src_h: self.fb_h,
                    src_stride: self.fb_stride,
                    title,
                    title_len: len,
                });
                self.focused = Some(i);
                self.cascade_n += 1;
                return;
            }
        }
    }

    #[inline]
    pub fn has_children(&self) -> bool {
        self.windows.iter().any(|w| w.is_some())
    }

    #[inline]
    pub fn any_surface_mapped(&self) -> bool {
        self.windows
            .iter()
            .any(|w| matches!(w, Some(win) if win.mapped))
    }
}

pub(super) const fn zeroed_surface_entry() -> compsys::SurfaceEntry {
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
