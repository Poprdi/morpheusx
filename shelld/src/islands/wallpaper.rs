use crate::islands::{ShellState, DESKTOP_RGB, raw_fill};

/// wallpaper island. renders a solid color desktop background into the
/// shell's surface buffer. compd blends it at z-layer 0 (background).
/// not fancy. doesn't need to be. it's a desktop.
pub fn tick(state: &mut ShellState) {
    if !state.wallpaper_dirty {
        return;
    }
    let (r, g, b) = DESKTOP_RGB;
    let px = state.pack(r, g, b);
    raw_fill(state.surface_ptr, state.fb_stride, 0, 0, state.fb_w, state.fb_h, px);
    state.wallpaper_dirty = false;
}
