use crate::islands::{raw_fill, ShellState};

/// Solid-color background; compd composites at z=0.
pub fn tick(state: &mut ShellState) {
    if !state.wallpaper_dirty {
        return;
    }
    let (r, g, b) = state.desktop_rgb;
    let px = state.pack(r, g, b);
    raw_fill(
        state.surface_ptr,
        state.fb_stride,
        0,
        0,
        state.fb_w,
        state.fb_h,
        px,
    );
    state.wallpaper_dirty = false;
}
