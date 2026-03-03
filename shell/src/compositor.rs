//! Window compositor — manages child process surfaces and composites them
//! onto the real framebuffer.  Handles input routing, window focus, and
//! title bar decorations.  Runs as a non-blocking event loop while child
//! processes are active.

extern crate alloc;

use libmorpheus::compositor as compsys;
use libmorpheus::{hw, io, mem, process};

use crate::fb::Framebuffer;
use crate::font;

// ── constants ──────────────────────────────────────────────────────────

/// Maximum number of simultaneously tracked child windows.
const MAX_WINDOWS: usize = 16;

/// Title bar height in pixels.
const TITLE_H: u32 = 22;

/// Border thickness in pixels.
const BORDER: u32 = 1;

/// Cascade offset for positioning new windows.
/// Set to 0 because child surfaces are full-framebuffer-resolution;
/// cascading full-size windows just clips content off-screen.
/// Focus switching (Ctrl+] or mouse click) is used to navigate.
const CASCADE_STEP: i32 = 0;

/// Title bar colours (packed for direct comparison, but we store RGB).
const TITLE_FOCUSED_RGB: (u8, u8, u8) = (0, 85, 0);
const TITLE_UNFOCUSED_RGB: (u8, u8, u8) = (40, 40, 40);
const TITLE_TEXT_RGB: (u8, u8, u8) = (255, 255, 255);
const BORDER_FOCUSED_RGB: (u8, u8, u8) = (0, 170, 0);
const BORDER_UNFOCUSED_RGB: (u8, u8, u8) = (85, 85, 85);
const DESKTOP_RGB: (u8, u8, u8) = (26, 26, 46);
const CURSOR_RGB: (u8, u8, u8) = (255, 255, 255);

// ── child window ───────────────────────────────────────────────────────

/// Tracks a running child process that may have a framebuffer surface.
struct ChildWindow {
    pid: u32,
    /// Write end of the pipe connected to the child's stdin (fd 0).
    pipe_wfd: u32,
    /// Pointer to the child's surface mapped into our address space.
    /// Null until the surface has been mapped.
    surface_ptr: *const u32,
    /// Whether the surface has been mapped.
    mapped: bool,
    /// Virtual address of the mapping in our (compositor) address space.
    /// Needed for munmap when the child exits to prevent UAF and VMA leak.
    surface_vaddr: u64,
    /// Number of pages in the surface mapping.
    surface_pages: u64,
    /// Window position on screen (content origin).
    x: i32,
    y: i32,
    /// Process name (for the title bar).
    title: [u8; 64],
    title_len: usize,
}

// ── compositor state ───────────────────────────────────────────────────

/// The compositor state: child windows, focus, mouse cursor.
pub struct Compositor {
    windows: [Option<ChildWindow>; MAX_WINDOWS],
    focused: Option<usize>,
    fb_w: u32,
    fb_h: u32,
    fb_stride: u32,
    is_bgrx: bool,
    cascade_n: i32,
    mouse_x: i32,
    mouse_y: i32,
    /// Set once the first compose+blit cycle runs.  Used to decide
    /// whether the shell console needs to be repainted after the loop.
    pub did_compose: bool,
    /// Scratch buffer for surface_list results.
    surface_buf: [compsys::SurfaceEntry; MAX_WINDOWS],
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
            did_compose: false,
            surface_buf: [zeroed_surface_entry(); MAX_WINDOWS],
        }
    }

    /// Register a freshly spawned child.  `pipe_wfd` is the write end of the
    /// pipe connected to the child's stdin.  `name` populates the title bar.
    pub fn add_child(&mut self, pid: u32, pipe_wfd: u32, name: &str) {
        for (i, slot) in self.windows.iter_mut().enumerate() {
            if slot.is_none() {
                let mut title = [0u8; 64];
                let len = name.len().min(63);
                title[..len].copy_from_slice(&name.as_bytes()[..len]);

                let x = CASCADE_STEP * (self.cascade_n % 5);
                // Place content origin below the title bar + border so the
                // title bar is always visible at the top of the screen.
                let y = (TITLE_H as i32 + BORDER as i32) + CASCADE_STEP * (self.cascade_n % 5);

                *slot = Some(ChildWindow {
                    pid,
                    pipe_wfd,
                    surface_ptr: core::ptr::null(),
                    mapped: false,
                    surface_vaddr: 0,
                    surface_pages: 0,
                    x,
                    y,
                    title,
                    title_len: len,
                });
                self.focused = Some(i);
                self.cascade_n += 1;
                return;
            }
        }
    }

    /// True if at least one child window is tracked.
    #[inline]
    pub fn has_children(&self) -> bool {
        self.windows.iter().any(|w| w.is_some())
    }

    /// True if at least one child has mapped a framebuffer surface.
    /// Non-graphical commands never map a surface, so this lets the
    /// compositor loop skip the expensive compose+blit cycle.
    #[inline]
    pub fn any_surface_mapped(&self) -> bool {
        self.windows
            .iter()
            .any(|w| matches!(w, Some(win) if win.mapped))
    }

    // ── input routing ──────────────────────────────────────────────────

    /// Write raw keyboard bytes to the focused child's stdin pipe.
    pub fn forward_keyboard(&self, data: &[u8]) {
        if let Some(idx) = self.focused {
            if let Some(win) = &self.windows[idx] {
                let _ = io::write_fd(win.pipe_wfd, data);
            }
        }
    }

    /// Read the global mouse accumulator and forward to the focused child.
    /// Also updates the on-screen cursor position.
    pub fn forward_mouse(&mut self) {
        let ms = hw::mouse_read();
        if ms.dx == 0 && ms.dy == 0 && ms.buttons == 0 {
            return;
        }

        self.mouse_x = (self.mouse_x + ms.dx as i32).clamp(0, self.fb_w as i32 - 1);
        self.mouse_y = (self.mouse_y + ms.dy as i32).clamp(0, self.fb_h as i32 - 1);

        // Alt+click on title bar → focus window (hit test)
        if (ms.buttons & 1) != 0 {
            self.hit_test_focus();
        }

        if let Some(idx) = self.focused {
            if let Some(win) = &self.windows[idx] {
                let _ = compsys::mouse_forward(win.pid, ms.dx, ms.dy, ms.buttons);
            }
        }
    }

    /// Cycle focus to the next window (Alt+Tab behaviour).
    pub fn cycle_focus(&mut self) {
        let active: alloc::vec::Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter_map(|(i, w)| if w.is_some() { Some(i) } else { None })
            .collect();

        if active.len() < 2 {
            return;
        }

        let cur = self.focused.unwrap_or(0);
        let pos = active.iter().position(|&i| i == cur).unwrap_or(0);
        let next = (pos + 1) % active.len();
        self.focused = Some(active[next]);
    }

    // ── surface management ─────────────────────────────────────────────

    /// Poll the kernel for new/updated surfaces and map any that haven't
    /// been mapped yet.
    pub fn update_surfaces(&mut self) {
        let count = compsys::surface_list(&mut self.surface_buf);

        for entry in &self.surface_buf[..count] {
            for win in self.windows.iter_mut().flatten() {
                if win.pid == entry.pid && !win.mapped {
                    if let Ok(ptr) = compsys::surface_map(entry.pid) {
                        win.surface_ptr = ptr as *const u32;
                        win.surface_vaddr = ptr as u64;
                        win.surface_pages = entry.pages;
                        win.mapped = true;
                    }
                }
            }
        }
    }

    // ── child lifecycle ────────────────────────────────────────────────

    /// Reap exited children (non-blocking).  Returns `Some(exit_code)` if
    /// the focused child exited, `None` otherwise.
    pub fn reap_exited(&mut self) -> Option<i32> {
        let mut focused_exit: Option<i32> = None;

        for (i, slot) in self.windows.iter_mut().enumerate() {
            let exited = if let Some(win) = slot {
                match process::try_wait(win.pid) {
                    Ok(Some(code)) => {
                        // Unmap the child's surface from our address space
                        // BEFORE it can be reallocated.  try_wait already
                        // reaped the child (freeing its physical pages), so
                        // the mapping is stale — remove it now.
                        if win.mapped && win.surface_vaddr != 0 && win.surface_pages != 0 {
                            let _ = mem::munmap(win.surface_vaddr, win.surface_pages);
                        }
                        let _ = libmorpheus::fs::close(win.pipe_wfd as usize);
                        if self.focused == Some(i) {
                            focused_exit = Some(code);
                        }
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };
            if exited {
                *slot = None;
            }
        }

        // Re-assign focus if the focused window was removed.
        if let Some(fi) = self.focused {
            if self.windows[fi].is_none() {
                self.focused = self.windows.iter().rposition(|w| w.is_some());
            }
        }

        focused_exit
    }

    // ── compositing ────────────────────────────────────────────────────

    /// Composite all child surfaces + decorations onto the real framebuffer.
    pub fn compose(&mut self, fb: &Framebuffer) {
        let fb_ptr = fb.as_ptr();

        // Desktop background.
        raw_fill(
            fb_ptr,
            self.fb_stride,
            0,
            0,
            self.fb_w,
            self.fb_h,
            self.pack(DESKTOP_RGB.0, DESKTOP_RGB.1, DESKTOP_RGB.2),
        );

        // Build draw order: non-focused first, focused last (on top).
        let mut order = [0u16; MAX_WINDOWS];
        let mut n = 0usize;
        for (i, w) in self.windows.iter().enumerate() {
            if w.is_some() && self.focused != Some(i) {
                order[n] = i as u16;
                n += 1;
            }
        }
        if let Some(fi) = self.focused {
            if self.windows[fi].is_some() {
                order[n] = fi as u16;
                n += 1;
            }
        }

        for &idx in &order[..n] {
            let idx = idx as usize;
            let is_focused = self.focused == Some(idx);
            if let Some(win) = &self.windows[idx] {
                self.draw_window(fb_ptr, win, is_focused);
            }
        }

        // Mouse cursor (simple 8×8 cross).
        self.draw_cursor(fb_ptr);

        // Clear dirty flags for all children.
        for win in self.windows.iter().flatten() {
            let _ = compsys::surface_dirty_clear(win.pid);
        }
    }

    // ── private drawing helpers ────────────────────────────────────────

    #[inline]
    fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        if self.is_bgrx {
            (b as u32) | ((g as u32) << 8) | ((r as u32) << 16)
        } else {
            (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
        }
    }

    fn draw_window(&self, fb_ptr: *mut u32, win: &ChildWindow, focused: bool) {
        let (tb_r, tb_g, tb_b) = if focused {
            TITLE_FOCUSED_RGB
        } else {
            TITLE_UNFOCUSED_RGB
        };
        let (br, bg, bb) = if focused {
            BORDER_FOCUSED_RGB
        } else {
            BORDER_UNFOCUSED_RGB
        };

        let outer_x = win.x - BORDER as i32;
        let outer_y = win.y - TITLE_H as i32 - BORDER as i32;

        // Total outer dimensions.
        let outer_w = self.fb_w + BORDER * 2;
        let outer_h = self.fb_h + TITLE_H + BORDER * 2;

        // Border — top.
        self.clip_fill(fb_ptr, outer_x, outer_y, outer_w, BORDER, br, bg, bb);
        // Border — left.
        self.clip_fill(
            fb_ptr,
            outer_x,
            outer_y + BORDER as i32,
            BORDER,
            outer_h - BORDER,
            br,
            bg,
            bb,
        );
        // Border — right.
        self.clip_fill(
            fb_ptr,
            outer_x + outer_w as i32 - BORDER as i32,
            outer_y + BORDER as i32,
            BORDER,
            outer_h - BORDER,
            br,
            bg,
            bb,
        );
        // Border — bottom.
        self.clip_fill(
            fb_ptr,
            outer_x,
            outer_y + outer_h as i32 - BORDER as i32,
            outer_w,
            BORDER,
            br,
            bg,
            bb,
        );

        // Title bar background.
        let tb_x = outer_x + BORDER as i32;
        let tb_y = outer_y + BORDER as i32;
        let tb_w = outer_w - BORDER * 2;
        self.clip_fill(fb_ptr, tb_x, tb_y, tb_w, TITLE_H, tb_r, tb_g, tb_b);

        // Title text.
        let title_str = core::str::from_utf8(&win.title[..win.title_len]).unwrap_or("?");
        let text_x = (tb_x + 6).max(0);
        let text_y = (tb_y + (TITLE_H as i32 - 16) / 2).max(0);
        self.draw_text(
            fb_ptr,
            text_x as u32,
            text_y as u32,
            title_str,
            TITLE_TEXT_RGB,
            (tb_r, tb_g, tb_b),
        );

        // Close button — [X].
        let close_x = (tb_x + tb_w as i32 - 32).max(0);
        self.draw_text(
            fb_ptr,
            close_x as u32,
            text_y as u32,
            "[X]",
            TITLE_TEXT_RGB,
            (tb_r, tb_g, tb_b),
        );

        // Child surface content.
        if win.mapped && !win.surface_ptr.is_null() {
            self.blit_surface(fb_ptr, win);
        }
    }

    /// Blit a child's per-process surface to the real framebuffer at the
    /// window's content position.  Clips to screen bounds.
    fn blit_surface(&self, fb_ptr: *mut u32, win: &ChildWindow) {
        let dst_x = win.x;
        let dst_y = win.y;
        let src_w = self.fb_w;
        let src_h = self.fb_h;
        let src_stride = self.fb_stride;
        let dst_stride = self.fb_stride;

        // Clip vertically.
        let y_start = if dst_y < 0 { (-dst_y) as u32 } else { 0 };
        let screen_y_start = (dst_y.max(0)) as u32;
        let rows = src_h
            .saturating_sub(y_start)
            .min(self.fb_h.saturating_sub(screen_y_start));

        // Clip horizontally.
        let x_start = if dst_x < 0 { (-dst_x) as u32 } else { 0 };
        let screen_x_start = (dst_x.max(0)) as u32;
        let cols = src_w
            .saturating_sub(x_start)
            .min(self.fb_w.saturating_sub(screen_x_start));

        if rows == 0 || cols == 0 {
            return;
        }

        unsafe {
            for row in 0..rows {
                let sy = y_start + row;
                let dy = screen_y_start + row;
                let src_off = (sy * src_stride + x_start) as usize;
                let dst_off = (dy * dst_stride + screen_x_start) as usize;
                core::ptr::copy_nonoverlapping(
                    win.surface_ptr.add(src_off),
                    fb_ptr.add(dst_off),
                    cols as usize,
                );
            }
        }
    }

    /// Fill a rectangle on the framebuffer, clipping to screen bounds.
    fn clip_fill(&self, fb_ptr: *mut u32, x: i32, y: i32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = ((x as i64 + w as i64).min(self.fb_w as i64)).max(0) as u32;
        let y1 = ((y as i64 + h as i64).min(self.fb_h as i64)).max(0) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let px = self.pack(r, g, b);
        raw_fill(fb_ptr, self.fb_stride, x0, y0, x1 - x0, y1 - y0, px);
    }

    /// Draw an ASCII string using the shell's built-in font.
    fn draw_text(
        &self,
        fb_ptr: *mut u32,
        x: u32,
        y: u32,
        text: &str,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        let fg_px = self.pack(fg.0, fg.1, fg.2);
        let bg_px = self.pack(bg.0, bg.1, bg.2);
        let font_w = 8u32;
        let _font_h = 16u32;

        for (ci, ch) in text.chars().enumerate() {
            let gx = x + ci as u32 * font_w;
            if gx + font_w > self.fb_w {
                break;
            }
            let glyph = font::get_glyph_or_space(ch);
            raw_glyph(
                fb_ptr,
                self.fb_stride,
                gx,
                y,
                glyph,
                fg_px,
                bg_px,
                self.fb_h,
            );
        }
    }

    /// Draw a small cross-hair cursor at (mouse_x, mouse_y).
    fn draw_cursor(&self, fb_ptr: *mut u32) {
        let cx = self.mouse_x;
        let cy = self.mouse_y;
        let px = self.pack(CURSOR_RGB.0, CURSOR_RGB.1, CURSOR_RGB.2);

        for d in -4i32..=4 {
            raw_put(fb_ptr, self.fb_stride, self.fb_w, self.fb_h, cx + d, cy, px);
            raw_put(fb_ptr, self.fb_stride, self.fb_w, self.fb_h, cx, cy + d, px);
        }
        // Outline for visibility on light backgrounds.
        let outline = self.pack(0, 0, 0);
        for d in [-5i32, 5] {
            raw_put(
                fb_ptr,
                self.fb_stride,
                self.fb_w,
                self.fb_h,
                cx + d,
                cy,
                outline,
            );
            raw_put(
                fb_ptr,
                self.fb_stride,
                self.fb_w,
                self.fb_h,
                cx,
                cy + d,
                outline,
            );
        }
    }

    /// Focus the window whose title bar contains (mouse_x, mouse_y).
    fn hit_test_focus(&mut self) {
        let mx = self.mouse_x;
        let my = self.mouse_y;

        // Iterate in reverse draw order (topmost first).
        let mut candidates: [Option<usize>; MAX_WINDOWS] = [None; MAX_WINDOWS];
        let mut cn = 0;

        // Focused window is drawn last (topmost), so check it first.
        if let Some(fi) = self.focused {
            candidates[cn] = Some(fi);
            cn += 1;
        }
        for (i, w) in self.windows.iter().enumerate().rev() {
            if w.is_some() && self.focused != Some(i) {
                candidates[cn] = Some(i);
                cn += 1;
            }
        }

        for &c in &candidates[..cn] {
            if let Some(idx) = c {
                if let Some(win) = &self.windows[idx] {
                    // Title bar area.
                    let tb_x = win.x - BORDER as i32;
                    let tb_y = win.y - TITLE_H as i32 - BORDER as i32;
                    let tb_w = self.fb_w as i32 + BORDER as i32 * 2;
                    let tb_h = TITLE_H as i32 + BORDER as i32;

                    if mx >= tb_x && mx < tb_x + tb_w && my >= tb_y && my < tb_y + tb_h {
                        self.focused = Some(idx);
                        return;
                    }

                    // Content area — also focus on click.
                    if mx >= win.x
                        && mx < win.x + self.fb_w as i32
                        && my >= win.y
                        && my < win.y + self.fb_h as i32
                    {
                        self.focused = Some(idx);
                        return;
                    }
                }
            }
        }
    }
}

// ── raw pixel helpers (no fb_mark_dirty) ───────────────────────────────

/// Fill a rectangle directly in the buffer (no syscall, no dirty flag).
#[inline]
fn raw_fill(buf: *mut u32, stride: u32, x: u32, y: u32, w: u32, h: u32, px: u32) {
    for row in y..y + h {
        let off = (row * stride + x) as usize;
        unsafe {
            let ptr = buf.add(off);
            for col in 0..w as usize {
                ptr.add(col).write(px);
            }
        }
    }
}

/// Put a single pixel (with bounds check).
#[inline]
fn raw_put(buf: *mut u32, stride: u32, w: u32, h: u32, x: i32, y: i32, px: u32) {
    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
        unsafe {
            buf.add((y as u32 * stride + x as u32) as usize).write(px);
        }
    }
}

/// Draw a single 8×16 glyph.
fn raw_glyph(
    buf: *mut u32,
    stride: u32,
    gx: u32,
    gy: u32,
    glyph: &[u8; 16],
    fg: u32,
    bg: u32,
    fb_h: u32,
) {
    for row in 0u32..16 {
        let py = gy + row;
        if py >= fb_h {
            break;
        }
        let bits = glyph[row as usize];
        let base = (py * stride + gx) as usize;
        for col in 0u32..8 {
            let is_fg = (bits >> (7 - col)) & 1 == 1;
            unsafe {
                buf.add(base + col as usize)
                    .write(if is_fg { fg } else { bg });
            }
        }
    }
}

/// Create a zeroed SurfaceEntry (const-friendly).
const fn zeroed_surface_entry() -> compsys::SurfaceEntry {
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

// ── public compositor loop ─────────────────────────────────────────────

/// Run the compositor event loop until all children exit.
///
/// Returns the exit code of the last focused child to exit.
pub fn compositor_loop(fb: &Framebuffer, comp: &mut Compositor) -> i32 {
    let mut last_status = 0i32;

    while comp.has_children() {
        // 1. Read keyboard (non-blocking).
        //    `read_stdin` parks the process if no data is available, which
        //    would freeze compositing, mouse, and child reaping.  Check
        //    availability first so we only read when bytes are waiting.
        let mut kb = [0u8; 32];
        let n = if io::stdin_available() > 0 {
            io::read_stdin(&mut kb)
        } else {
            0
        };

        if n > 0 {
            // Ctrl+] (0x1D) cycles focus between windows.  All other bytes
            // are forwarded to the focused child's stdin pipe.  We must not
            // drop non-trigger bytes that happen to share the same read
            // buffer, so we forward everything except the trigger itself.
            let mut has_cycle = false;
            for i in 0..n {
                if kb[i] == 0x1D {
                    has_cycle = true;
                    kb[i] = 0; // mark consumed (will be filtered out below)
                }
            }
            if has_cycle {
                comp.cycle_focus();
            }
            // Forward remaining bytes (skip consumed 0x1D slots).
            let mut fwd = [0u8; 32];
            let mut fi = 0;
            for i in 0..n {
                if kb[i] != 0 || (!has_cycle) {
                    fwd[fi] = kb[i];
                    fi += 1;
                }
            }
            if fi > 0 {
                comp.forward_keyboard(&fwd[..fi]);
            }
        }

        // 2. Mouse.
        comp.forward_mouse();

        // 3. Map any new surfaces.
        comp.update_surfaces();

        // 4. Composite + present.  Only compose if at least one child
        //    has a mapped surface — non-graphical commands (ls, echo, etc.)
        //    exit quickly without ever mapping a FB, so we avoid the visual
        //    flash of the desktop background.
        //
        // Use fb_present() for delta rendering (only changed pixels).
        // fb_blit() would do full-screen memcpy which causes tearing.
        if comp.any_surface_mapped() {
            comp.compose(fb);
            let _ = hw::fb_present();
            comp.did_compose = true;
        }

        // 5. Reap exited children (AFTER compositing to avoid use-after-free).
        if let Some(code) = comp.reap_exited() {
            last_status = code;
        }

        // 6. Yield to let children run.
        process::yield_cpu();
    }

    last_status
}
