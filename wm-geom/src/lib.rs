//! Window-management geometry — the pure spatial math compd uses to hit-test a pointer, move and
//! resize the focused window, and pick a cursor shape. Extracted from compd into its own crate so
//! the WM-interaction domain has a single home with host-runnable tests.
//!
//! Why tests live here and not on a boot: the headless QEMU this kernel is exercised in has no
//! working pointer device — the PS/2 mouse driver is a kernel stub (`bootloader desktop owns PS/2
//! mouse I/O`) and the emulated xHCI controller fails to initialise — so the move/resize/hit-test
//! path cannot be driven by a real mouse on a boot. This crate is `no_std` for the compd build and
//! links `std` only under `cfg(test)` (mirroring the `keymap` crate), so the exact pixel math is
//! pinned by deterministic unit tests. compd owns the window list, z-order, focus and IPC; this
//! crate owns only the per-window geometry.
#![cfg_attr(not(test), no_std)]

/// A rectangle in framebuffer pixels. For a window this is the *content* rect — its top-left is the
/// client area; the title bar and 1px border sit above and around it (see [`Chrome`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Chrome metrics in pixels. `close_off`/`close_w` describe the `[X]` button as an inset from the
/// title bar's right edge: the hit cell is `[right - close_off, right - close_off + close_w)`.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub title_h: i32,
    pub border: i32,
    pub grip: i32,
    pub close_off: i32,
    pub close_w: i32,
}

/// The part of a window a point falls on. Priority when overlapping: Close > Resize > Title >
/// Content (a click on the grip resizes even though it sits over the content's bottom-right).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Title,
    Close,
    Resize,
    Content,
}

/// An in-progress drag captured on press, so the interaction (and the cursor) stays stable for the
/// whole drag even if the pointer briefly leaves the handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capture {
    Move,
    Resize,
}

/// The cursor glyph compd draws. Reflects what a press would do, so window interaction reads clearly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorShape {
    Arrow,
    Move,
    Resize,
}

/// Classify a point against ONE window's chrome, or `None` if it misses the window's outer box.
/// Behaviour-identical to compd's former inline hit-test: the outer box spans the border + title
/// bar; the close button, then the grip, then the title bar, then the content are tested in turn,
/// and a point inside the outer box but on bare border pixels classifies as `None`.
pub fn classify(win: Rect, c: Chrome, mx: i32, my: i32) -> Option<Region> {
    let outer_x = win.x - c.border;
    let outer_y = win.y - c.title_h - c.border;
    let outer_w = win.w + c.border * 2;
    let outer_h = win.h + c.title_h + c.border * 2;

    if mx < outer_x || mx >= outer_x + outer_w || my < outer_y || my >= outer_y + outer_h {
        return None;
    }

    let tb_x = outer_x + c.border; // == win.x
    let tb_y = outer_y + c.border; // == win.y - title_h
    let tb_w = win.w;

    // Close button: an inset cell at the right of the title bar.
    let close_x = tb_x + tb_w - c.close_off;
    if my >= tb_y && my < tb_y + c.title_h && mx >= close_x && mx < close_x + c.close_w {
        return Some(Region::Close);
    }

    // Bottom-right resize grip (square, side = grip).
    let resize_x = win.x + win.w - c.grip;
    let resize_y = win.y + win.h - c.grip;
    if mx >= resize_x && my >= resize_y {
        return Some(Region::Resize);
    }

    // The rest of the title bar.
    if my >= tb_y && my < tb_y + c.title_h {
        return Some(Region::Title);
    }

    // The client content area.
    if mx >= win.x && mx < win.x + win.w && my >= win.y && my < win.y + win.h {
        return Some(Region::Content);
    }

    None
}

/// The cursor shape: a live drag (`capture`) wins; otherwise the hovered region drives it. A title
/// hover (or a move drag) → move; a grip hover (or a resize drag) → resize; everything else → arrow.
pub fn cursor_shape(capture: Option<Capture>, hover: Option<Region>) -> CursorShape {
    match capture {
        Some(Capture::Move) => CursorShape::Move,
        Some(Capture::Resize) => CursorShape::Resize,
        None => match hover {
            Some(Region::Title) => CursorShape::Move,
            Some(Region::Resize) => CursorShape::Resize,
            _ => CursorShape::Arrow,
        },
    }
}

/// New window top-left when dragging the title bar: the desired position `(nx, ny)` (pointer minus
/// the grab offset) clamped so the window stays reachable — `x ∈ [0, fb_w - w]` and `y ∈ [title_h,
/// fb_h - h]`, with the title bar never allowed above `title_h` so it stays on-screen.
pub fn clamp_move(w: i32, h: i32, fb_w: i32, fb_h: i32, title_h: i32, nx: i32, ny: i32) -> (i32, i32) {
    let max_x = (fb_w - w).max(0);
    let max_y = (fb_h - h).max(title_h);
    (nx.clamp(0, max_x), ny.clamp(title_h, max_y))
}

/// Round a pixel extent DOWN to a whole number of cells. A window's content is blitted 1:1, so its
/// size must be a cell multiple or the client would leave a partial edge cell unpainted.
#[inline]
pub fn snap_cells(px: i32, cell: i32) -> i32 {
    let cell = cell.max(1);
    (px.max(0) / cell) * cell
}

/// New `(w, h)` when dragging the corner grip: the start size plus the pointer delta `(dx, dy)`,
/// clamped to `[min, fb - window-origin]` so the window stays within the framebuffer and above a
/// usable minimum, then snapped DOWN to whole cells.
#[allow(clippy::too_many_arguments)]
pub fn snap_resize(
    start_w: i32,
    start_h: i32,
    dx: i32,
    dy: i32,
    win_x: i32,
    win_y: i32,
    fb_w: i32,
    fb_h: i32,
    cell_w: i32,
    cell_h: i32,
    min_w: i32,
    min_h: i32,
) -> (i32, i32) {
    let max_w = (fb_w - win_x.max(0)).max(min_w);
    let max_h = (fb_h - win_y.max(0)).max(min_h);
    let nw = (start_w + dx).clamp(min_w, max_w);
    let nh = (start_h + dy).clamp(min_h, max_h);
    (snap_cells(nw, cell_w), snap_cells(nh, cell_h))
}

// ── Keyboard-first window management ───────────────────────────────────────────────────────────
//
// The DE is keyboard-driven, so the focused window must be move/resize/closeable without a pointer
// (which this kernel lacks headless anyway). compd intercepts a chord in the established Ctrl+Alt
// window-management namespace (already home to Ctrl+Alt+X = spawn shell) and asks this crate to map
// it to a command, then to apply the geometry. Keeping the chord table AND the step math here means
// the whole keyboard-WM behaviour is host-tested — the same code the boot runs.
//
// PS/2 Set-1 scancodes for the vim direction cluster + the close key. compd hands us the true
// scancode its decoder recovered on the key's release edge.
pub const SC_H: u8 = 0x23;
pub const SC_J: u8 = 0x24;
pub const SC_K: u8 = 0x25;
pub const SC_L: u8 = 0x26;
pub const SC_C: u8 = 0x2E;

// PS/2 Set-1 scancodes for the number row `1`..`9` — the tiling/snap grid (see [`snap_zone_for`]).
pub const SC_1: u8 = 0x02;
pub const SC_2: u8 = 0x03;
pub const SC_3: u8 = 0x04;
pub const SC_4: u8 = 0x05;
pub const SC_5: u8 = 0x06;
pub const SC_6: u8 = 0x07;
pub const SC_7: u8 = 0x08;
pub const SC_8: u8 = 0x09;
pub const SC_9: u8 = 0x0A;

/// How far a single Move chord nudges the window, and how much a single Resize chord grows/shrinks
/// it. 32px = 4 columns / 2 rows of the 8×16 cell grid — a brisk but controllable step. Resize
/// deltas are cell-snapped by [`snap_resize`], so a window stays a whole number of cells.
pub const MOVE_STEP: i32 = 32;
pub const RESIZE_STEP_W: i32 = 32;
pub const RESIZE_STEP_H: i32 = 32;

/// A cardinal direction — the HJKL cluster, vim order (H=left, J=down, K=up, L=right).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// A screen region the focused window can be SNAPPED (tiled) to fill. The nine zones map onto the
/// number-row grid like a numpad: the four corners are quadrants, the edge centres are halves, and
/// the centre maximizes (a toggle — compd restores the pre-maximize geometry on the next press).
/// All zones fill the *work area* (the framebuffer minus the bottom taskbar), so a tiled window
/// sits flush above the panel rather than sliding under it the way a free Move/Resize may.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapZone {
    Maximize,
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// A keyboard window-management command on the focused window. Move nudges it; Resize grows the
/// width (Left/Right) or height (Up/Down) edge; Snap tiles it to a work-area region; Close asks
/// compd to terminate the client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WmCommand {
    Move(Dir),
    Resize(Dir),
    Snap(SnapZone),
    Close,
}

/// Map a number-row scancode to its snap zone, laid out as a numpad-spatial 3×3 grid:
///
/// ```text
///   7 8 9      ┌───────────┬───────────┬───────────┐
///   4 5 6  →   │ top-left  │ top-half  │ top-right │   (8 = top half, 2 = bottom half,
///   1 2 3      │ left-half │ maximize  │ right-half│    4 = left half, 6 = right half,
///              │ bot-left  │ bot-half  │ bot-right │    5 = maximize/restore toggle)
///              └───────────┴───────────┴───────────┘
/// ```
pub fn snap_zone_for(scancode: u8) -> Option<SnapZone> {
    Some(match scancode {
        SC_7 => SnapZone::TopLeft,
        SC_8 => SnapZone::TopHalf,
        SC_9 => SnapZone::TopRight,
        SC_4 => SnapZone::LeftHalf,
        SC_5 => SnapZone::Maximize,
        SC_6 => SnapZone::RightHalf,
        SC_1 => SnapZone::BottomLeft,
        SC_2 => SnapZone::BottomHalf,
        SC_3 => SnapZone::BottomRight,
        _ => return None,
    })
}

/// Map a decoded key (scancode + effective modifiers) to a window-management command, or `None` if
/// it isn't a WM chord (so compd forwards it to the client untouched). The namespace is Ctrl+Alt:
/// the bare cluster moves, +Shift resizes, and Ctrl+Alt+C closes. AltGr (right Alt) is a separate
/// modifier in the decoder, so character-composing chords never collide with these.
pub fn wm_command(scancode: u8, ctrl: bool, alt: bool, shift: bool) -> Option<WmCommand> {
    if !(ctrl && alt) {
        return None;
    }
    // Tiling/snap on the number-row grid. Shift-independent (a client never binds Ctrl+Alt+digit,
    // with or without Shift), and the digits are disjoint from the HJKL/C cluster below.
    if let Some(zone) = snap_zone_for(scancode) {
        return Some(WmCommand::Snap(zone));
    }
    let dir = match scancode {
        SC_H => Some(Dir::Left),
        SC_L => Some(Dir::Right),
        SC_K => Some(Dir::Up),
        SC_J => Some(Dir::Down),
        _ => None,
    };
    if shift {
        dir.map(WmCommand::Resize)
    } else if let Some(d) = dir {
        Some(WmCommand::Move(d))
    } else if scancode == SC_C {
        Some(WmCommand::Close)
    } else {
        None
    }
}

/// Apply a Move/Resize command to the focused window's content rect, reusing the same clamp/snap
/// math the mouse drag uses (so keyboard and pointer leave a window in identical, valid states).
/// Close returns the rect unchanged (compd terminates the client; there is no geometry change).
#[allow(clippy::too_many_arguments)]
pub fn apply_command(
    win: Rect,
    cmd: WmCommand,
    fb_w: i32,
    fb_h: i32,
    title_h: i32,
    cell_w: i32,
    cell_h: i32,
    min_w: i32,
    min_h: i32,
) -> Rect {
    match cmd {
        WmCommand::Close => win,
        // Snap is resolved by the caller via [`snap_rect`] — it needs the panel height, border and
        // the maximize/restore saved-rect state, none of which this stateless mover has. Returning
        // the rect unchanged keeps the match exhaustive without a spurious geometry change.
        WmCommand::Snap(_) => win,
        WmCommand::Move(dir) => {
            let (dx, dy) = match dir {
                Dir::Left => (-MOVE_STEP, 0),
                Dir::Right => (MOVE_STEP, 0),
                Dir::Up => (0, -MOVE_STEP),
                Dir::Down => (0, MOVE_STEP),
            };
            let (x, y) = clamp_move(win.w, win.h, fb_w, fb_h, title_h, win.x + dx, win.y + dy);
            Rect { x, y, ..win }
        },
        WmCommand::Resize(dir) => {
            let (dw, dh) = match dir {
                Dir::Left => (-RESIZE_STEP_W, 0),
                Dir::Right => (RESIZE_STEP_W, 0),
                Dir::Up => (0, -RESIZE_STEP_H),
                Dir::Down => (0, RESIZE_STEP_H),
            };
            let (w, h) = snap_resize(
                win.w, win.h, dw, dh, win.x, win.y, fb_w, fb_h, cell_w, cell_h, min_w, min_h,
            );
            Rect { w, h, ..win }
        },
    }
}

/// Compute the CONTENT rect a window takes when snapped to `zone`. The zones tile the *work area*
/// — the framebuffer minus the bottom taskbar (`panel_h`) — so a snapped window sits flush above
/// the panel. Each zone's outer box (border + title bar + content) is fitted inside the work area,
/// then the content rect is derived by insetting the chrome: the title bar (`title_h` + `border`)
/// sits above the content, a `border` frames the other three sides. The content's width/height are
/// snapped DOWN to whole cells (the client blits 1:1) and floored at `min_w`/`min_h`.
#[allow(clippy::too_many_arguments)]
pub fn snap_rect(
    zone: SnapZone,
    fb_w: i32,
    fb_h: i32,
    panel_h: i32,
    title_h: i32,
    border: i32,
    cell_w: i32,
    cell_h: i32,
    min_w: i32,
    min_h: i32,
) -> Rect {
    let wa_w = fb_w.max(1);
    // Reserve the panel, but never let the work area collapse below a single chrome+cell box.
    let wa_h = (fb_h - panel_h).max(title_h + border * 2 + cell_h);

    // Outer box (ox, oy, ow, oh) of the zone within the work area (top-left origin = fb top-left,
    // since the panel only reserves the bottom).
    let (ox, oy, ow, oh) = zone_outer_box(zone, wa_w, wa_h);

    let x = ox + border;
    let y = oy + title_h + border;
    let w = snap_cells((ow - border * 2).max(min_w), cell_w);
    let h = snap_cells((oh - (title_h + border * 2)).max(min_h), cell_h);
    Rect { x, y, w, h }
}

/// The zone's outer box `(ox, oy, ow, oh)` within a `wa_w × wa_h` work area — the raw region a
/// snapped window's chrome+content fills, before any chrome inset. Shared by [`snap_rect`] (which
/// then insets the chrome) and [`snap_zone_outer`] (which highlights this box as the drag preview),
/// so the two can never describe different regions for the same zone.
fn zone_outer_box(zone: SnapZone, wa_w: i32, wa_h: i32) -> (i32, i32, i32, i32) {
    let mid_x = wa_w / 2;
    let mid_y = wa_h / 2;
    match zone {
        SnapZone::Maximize => (0, 0, wa_w, wa_h),
        SnapZone::LeftHalf => (0, 0, mid_x, wa_h),
        SnapZone::RightHalf => (mid_x, 0, wa_w - mid_x, wa_h),
        SnapZone::TopHalf => (0, 0, wa_w, mid_y),
        SnapZone::BottomHalf => (0, mid_y, wa_w, wa_h - mid_y),
        SnapZone::TopLeft => (0, 0, mid_x, mid_y),
        SnapZone::TopRight => (mid_x, 0, wa_w - mid_x, mid_y),
        SnapZone::BottomLeft => (0, mid_y, mid_x, wa_h - mid_y),
        SnapZone::BottomRight => (mid_x, mid_y, wa_w - mid_x, wa_h - mid_y),
    }
}

/// The outer-box [`Rect`] a `zone` fills within the work area (the framebuffer minus the bottom
/// `panel_h` taskbar) — what a drag-to-snap *preview* highlights so the user sees where the window
/// will land before releasing. This is the full chrome+content region (the same box [`snap_rect`]
/// insets the window chrome into), not the inset content rect, because the preview stands in for the
/// whole window. Returned in framebuffer pixels with a top-left origin.
pub fn snap_zone_outer(zone: SnapZone, fb_w: i32, fb_h: i32, panel_h: i32) -> Rect {
    let wa_w = fb_w.max(1);
    let wa_h = (fb_h - panel_h).max(1);
    let (ox, oy, ow, oh) = zone_outer_box(zone, wa_w, wa_h);
    Rect { x: ox, y: oy, w: ow, h: oh }
}

/// Map the pointer position during a title-bar drag to the snap zone it would tile to on release —
/// the canonical "Aero Snap" gesture. `None` away from any edge (the window just drops where it was
/// dragged). The trigger geometry, in framebuffer pixels:
///
/// - within `edge` of the **left** or **right** screen edge → that side's half, UNLESS the pointer
///   is also within `corner` of the work-area top or bottom, which makes it that side's quadrant;
/// - within `edge` of the **top** edge (and not in a side band) → maximize.
///
/// The bottom screen edge is the taskbar's; compd routes a pointer there to the panel and never
/// reaches this, so bottom snapping is via the lower portion of the left/right bands (the `corner`
/// band sits just above the panel, where the pointer is still over the work area). `panel_h` defines
/// the work-area bottom against which the bottom corners are measured.
pub fn edge_snap_zone(
    mx: i32,
    my: i32,
    fb_w: i32,
    fb_h: i32,
    panel_h: i32,
    edge: i32,
    corner: i32,
) -> Option<SnapZone> {
    let work_h = (fb_h - panel_h).max(1);
    let near_left = mx < edge;
    let near_right = mx >= fb_w - edge;
    let near_top = my < edge;
    let top_corner = my < corner;
    let bot_corner = my >= work_h - corner;

    if near_left {
        return Some(if top_corner {
            SnapZone::TopLeft
        } else if bot_corner {
            SnapZone::BottomLeft
        } else {
            SnapZone::LeftHalf
        });
    }
    if near_right {
        return Some(if top_corner {
            SnapZone::TopRight
        } else if bot_corner {
            SnapZone::BottomRight
        } else {
            SnapZone::RightHalf
        });
    }
    if near_top {
        return Some(SnapZone::Maximize);
    }
    None
}

// ── Focus cycling + double-click (C2) ────────────────────────────────────────────────────────────

/// Whether a fresh title-bar press is the SECOND click of a double-click: the previous press
/// (`last` = `(timestamp_ms, window_idx)`) was on the SAME window and within `threshold_ms`. A
/// `None` history — no prior press, or one already consumed — is never a double. Time runs through
/// `saturating_sub`, so a non-monotonic clock can only ever shrink the gap (defensive, never panics).
/// Pure so the timing predicate is host-tested rather than only exercised on a boot with a pointer.
pub fn is_double_click(
    last: Option<(u64, usize)>,
    idx: usize,
    now_ms: u64,
    threshold_ms: u64,
) -> bool {
    match last {
        Some((t, i)) => i == idx && now_ms.saturating_sub(t) <= threshold_ms,
        None => false,
    }
}

/// The next window slot to focus when cycling (Alt+Tab forward, Shift+Alt+Tab reverse, Ctrl+]
/// forward). Steps around a ring of `n` slots from `current` — `+1` forward, `-1` when `reverse` —
/// and returns the first slot for which `focusable(idx)` holds. The scan covers all `n` slots and
/// wraps, so it lands back on `current` only when nothing else qualifies, and returns `None` only
/// when no slot is focusable at all. With no current focus the forward scan begins at slot 0 and the
/// reverse scan at slot `n-1`. Pure, so the wrap-around order is pinned by host tests, not a boot.
pub fn next_focus(
    current: Option<usize>,
    n: usize,
    reverse: bool,
    focusable: impl Fn(usize) -> bool,
) -> Option<usize> {
    if n == 0 {
        return None;
    }
    // Anchor the scan just off `current`: the first candidate is anchor±1. With no current focus,
    // anchor n-1 (forward) / 0 (reverse) makes that first candidate land on slot 0 / n-1.
    let anchor = match (current, reverse) {
        (Some(c), _) => c % n,
        (None, false) => n - 1,
        (None, true) => 0,
    };
    for offset in 1..=n {
        let idx = if reverse {
            (anchor + n - (offset % n)) % n
        } else {
            (anchor + offset) % n
        };
        if focusable(idx) {
            return Some(idx);
        }
    }
    None
}

/// What activating a window from its taskbar chip should do, given the window's current state. This
/// is the canonical taskbar policy a real desktop follows: a *minimized* window restores (and takes
/// focus); clicking the chip of the window that is *already focused* and visible minimizes it (the
/// active button toggles the window away); any *other* visible window is simply focused + raised.
/// Pure, so the toggle policy is host-tested rather than only exercised on a boot with a pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipAction {
    /// Un-minimize the window and give it focus (it was hidden).
    Restore,
    /// Hide the window — it is the currently-focused, visible one (the active-button toggle).
    Minimize,
    /// Bring an already-visible, unfocused window to the front and focus it.
    Focus,
}

/// Decide what a taskbar-chip activation does from the target window's `(is_minimized, is_focused)`
/// state. See [`ChipAction`] for the policy. The compositor owns the actual focus/visibility edit;
/// this only names the intent so the decision is unit-tested off-hardware.
pub fn chip_action(is_minimized: bool, is_focused: bool) -> ChipAction {
    if is_minimized {
        ChipAction::Restore
    } else if is_focused {
        ChipAction::Minimize
    } else {
        ChipAction::Focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 800×496 window whose content top-left is at (20, 42); title bar = 22px, border = 1px,
    // grip = 14px, close button = 30px wide inset 34px from the right.
    const C: Chrome = Chrome {
        title_h: 22,
        border: 1,
        grip: 14,
        close_off: 34,
        close_w: 30,
    };
    const WIN: Rect = Rect {
        x: 20,
        y: 42,
        w: 800,
        h: 496,
    };

    #[test]
    fn point_outside_outer_box_is_none() {
        // Far away, and just left of the 1px border.
        assert_eq!(classify(WIN, C, 5, 5), None);
        assert_eq!(classify(WIN, C, WIN.x - C.border - 1, WIN.y), None);
    }

    #[test]
    fn title_bar_classifies_as_title() {
        // Middle of the bar, above the content top.
        let ty = WIN.y - C.title_h / 2; // inside [y-22, y)
        assert_eq!(classify(WIN, C, WIN.x + 200, ty), Some(Region::Title));
    }

    #[test]
    fn close_button_classifies_as_close_and_beats_title() {
        let right = WIN.x + WIN.w;
        let cx = right - C.close_off + C.close_w / 2; // centre of the [X] cell
        let cy = WIN.y - C.title_h / 2;
        assert_eq!(classify(WIN, C, cx, cy), Some(Region::Close));
        // One pixel left of the close cell is still the title bar.
        assert_eq!(
            classify(WIN, C, right - C.close_off - 1, cy),
            Some(Region::Title)
        );
    }

    #[test]
    fn grip_classifies_as_resize_and_beats_content() {
        // Bottom-right inside the grip square (which overlaps the content corner).
        let gx = WIN.x + WIN.w - C.grip / 2;
        let gy = WIN.y + WIN.h - C.grip / 2;
        assert_eq!(classify(WIN, C, gx, gy), Some(Region::Resize));
        // Just inside-and-up-left of the grip is plain content.
        assert_eq!(
            classify(WIN, C, WIN.x + WIN.w - C.grip - 1, WIN.y + WIN.h - C.grip - 1),
            Some(Region::Content)
        );
    }

    #[test]
    fn content_classifies_as_content() {
        assert_eq!(classify(WIN, C, WIN.x + 10, WIN.y + 10), Some(Region::Content));
    }

    #[test]
    fn cursor_shape_capture_wins_over_hover() {
        // Dragging keeps the shape even if the pointer is over content.
        assert_eq!(
            cursor_shape(Some(Capture::Move), Some(Region::Content)),
            CursorShape::Move
        );
        assert_eq!(
            cursor_shape(Some(Capture::Resize), None),
            CursorShape::Resize
        );
    }

    #[test]
    fn cursor_shape_follows_hover_when_idle() {
        assert_eq!(cursor_shape(None, Some(Region::Title)), CursorShape::Move);
        assert_eq!(cursor_shape(None, Some(Region::Resize)), CursorShape::Resize);
        assert_eq!(cursor_shape(None, Some(Region::Content)), CursorShape::Arrow);
        assert_eq!(cursor_shape(None, Some(Region::Close)), CursorShape::Arrow);
        assert_eq!(cursor_shape(None, None), CursorShape::Arrow);
    }

    #[test]
    fn move_clamps_into_the_framebuffer() {
        let (fb_w, fb_h, th) = (1280, 800, 22);
        // Dragging up-left past the origin pins to (0, title_h).
        assert_eq!(clamp_move(800, 496, fb_w, fb_h, th, -50, -50), (0, 22));
        // Dragging down-right past the edge pins to (fb - size).
        assert_eq!(
            clamp_move(800, 496, fb_w, fb_h, th, 9999, 9999),
            (fb_w - 800, fb_h - 496)
        );
        // A reachable position passes through unchanged.
        assert_eq!(clamp_move(800, 496, fb_w, fb_h, th, 200, 150), (200, 150));
    }

    #[test]
    fn resize_snaps_to_cells_and_clamps_to_min() {
        // Shrinking by a non-cell delta snaps DOWN to whole 8×16 cells.
        let (w, h) = snap_resize(800, 496, -13, -17, 20, 42, 1280, 800, 8, 16, 160, 120);
        assert_eq!((w, h), (snap_cells(787, 8), snap_cells(479, 16)));
        assert_eq!((w, h), (784, 464));
        // Shrinking far past the minimum pins to the minimum, snapped to cells: 160 is already
        // 20 cells of 8px; the 120px floor snaps DOWN to 112 (7 cells of 16px).
        let (w2, h2) = snap_resize(800, 496, -9999, -9999, 20, 42, 1280, 800, 8, 16, 160, 120);
        assert_eq!((w2, h2), (160, 112));
    }

    #[test]
    fn resize_clamps_to_framebuffer_extent_from_window_origin() {
        // Growing without bound is capped at fb minus the window origin, then cell-snapped.
        let (w, h) = snap_resize(800, 496, 9999, 9999, 20, 42, 1280, 800, 8, 16, 160, 120);
        assert_eq!(w, snap_cells(1280 - 20, 8)); // 1260 → 1256
        assert_eq!(h, snap_cells(800 - 42, 16)); // 758 → 752
        assert_eq!((w, h), (1256, 752));
    }

    #[test]
    fn chord_table_maps_ctrl_alt_cluster() {
        // Bare Ctrl+Alt + HJKL moves in vim directions.
        assert_eq!(wm_command(SC_H, true, true, false), Some(WmCommand::Move(Dir::Left)));
        assert_eq!(wm_command(SC_L, true, true, false), Some(WmCommand::Move(Dir::Right)));
        assert_eq!(wm_command(SC_K, true, true, false), Some(WmCommand::Move(Dir::Up)));
        assert_eq!(wm_command(SC_J, true, true, false), Some(WmCommand::Move(Dir::Down)));
        // +Shift resizes the matching edge.
        assert_eq!(wm_command(SC_H, true, true, true), Some(WmCommand::Resize(Dir::Left)));
        assert_eq!(wm_command(SC_J, true, true, true), Some(WmCommand::Resize(Dir::Down)));
        // Ctrl+Alt+C closes (only without Shift).
        assert_eq!(wm_command(SC_C, true, true, false), Some(WmCommand::Close));
        assert_eq!(wm_command(SC_C, true, true, true), None);
    }

    #[test]
    fn chord_table_ignores_non_wm_keys_and_missing_modifiers() {
        // Missing either modifier ⇒ not a WM chord (forwarded to the client).
        assert_eq!(wm_command(SC_H, false, true, false), None);
        assert_eq!(wm_command(SC_H, true, false, false), None);
        // A letter outside the cluster with the full modifier set is still not ours (e.g. X = the
        // spawn-shell hotkey compd owns separately).
        assert_eq!(wm_command(0x2D, true, true, false), None);
    }

    #[test]
    fn apply_move_nudges_and_clamps() {
        let win = Rect { x: 100, y: 100, w: 800, h: 496 };
        let moved = apply_command(win, WmCommand::Move(Dir::Right), 1280, 800, 22, 8, 16, 160, 120);
        assert_eq!(moved, Rect { x: 132, y: 100, w: 800, h: 496 });
        // Nudging left from the edge pins x at 0 (size unchanged).
        let at_left = Rect { x: 10, y: 100, w: 800, h: 496 };
        let pinned = apply_command(at_left, WmCommand::Move(Dir::Left), 1280, 800, 22, 8, 16, 160, 120);
        assert_eq!(pinned, Rect { x: 0, y: 100, w: 800, h: 496 });
    }

    #[test]
    fn apply_resize_grows_width_and_snaps_cells() {
        let win = Rect { x: 20, y: 42, w: 800, h: 496 };
        let wider = apply_command(win, WmCommand::Resize(Dir::Right), 1280, 800, 22, 8, 16, 160, 120);
        assert_eq!(wider, Rect { x: 20, y: 42, w: 832, h: 496 }); // +32px = +4 cells, origin fixed.
        let taller = apply_command(win, WmCommand::Resize(Dir::Down), 1280, 800, 22, 8, 16, 160, 120);
        assert_eq!(taller, Rect { x: 20, y: 42, w: 800, h: 528 }); // +32px = +2 rows.
        // Shrinking past the minimum pins to the cell-snapped floor (160px = 20 cells; 120 → 112).
        let tiny = Rect { x: 20, y: 42, w: 160, h: 128 };
        let shrunk = apply_command(tiny, WmCommand::Resize(Dir::Left), 1280, 800, 22, 8, 16, 160, 120);
        assert_eq!(shrunk.w, 160);
    }

    #[test]
    fn apply_close_leaves_geometry_unchanged() {
        let win = Rect { x: 20, y: 42, w: 800, h: 496 };
        assert_eq!(apply_command(win, WmCommand::Close, 1280, 800, 22, 8, 16, 160, 120), win);
    }

    // ── Tiling / snap (C4) ──────────────────────────────────────────────────────────────────────

    #[test]
    fn snap_chord_grid_maps_the_number_row() {
        // The 3×3 numpad-spatial grid, all under Ctrl+Alt (Shift-independent).
        let c = |sc| wm_command(sc, true, true, false);
        let cs = |sc| wm_command(sc, true, true, true); // +Shift still snaps.
        assert_eq!(c(SC_7), Some(WmCommand::Snap(SnapZone::TopLeft)));
        assert_eq!(c(SC_8), Some(WmCommand::Snap(SnapZone::TopHalf)));
        assert_eq!(c(SC_9), Some(WmCommand::Snap(SnapZone::TopRight)));
        assert_eq!(c(SC_4), Some(WmCommand::Snap(SnapZone::LeftHalf)));
        assert_eq!(c(SC_5), Some(WmCommand::Snap(SnapZone::Maximize)));
        assert_eq!(cs(SC_5), Some(WmCommand::Snap(SnapZone::Maximize)));
        assert_eq!(c(SC_6), Some(WmCommand::Snap(SnapZone::RightHalf)));
        assert_eq!(c(SC_1), Some(WmCommand::Snap(SnapZone::BottomLeft)));
        assert_eq!(c(SC_2), Some(WmCommand::Snap(SnapZone::BottomHalf)));
        assert_eq!(c(SC_3), Some(WmCommand::Snap(SnapZone::BottomRight)));
        // Missing a modifier ⇒ not a snap (the digit forwards to the client).
        assert_eq!(wm_command(SC_5, false, true, false), None);
        assert_eq!(wm_command(SC_5, true, false, false), None);
        // A snap command leaves geometry to the caller's snap_rect (apply_command passes it through).
        let win = Rect { x: 20, y: 42, w: 800, h: 496 };
        assert_eq!(
            apply_command(win, WmCommand::Snap(SnapZone::Maximize), 1280, 800, 22, 8, 16, 160, 120),
            win
        );
    }

    // Common chrome/grid for the snap-geometry tests: 1280×800 fb, 30px panel, 22px title, 1px
    // border, 8×16 cells. Work area = 1280×770; mid = (640, 385).
    const FB_W: i32 = 1280;
    const FB_H: i32 = 800;
    const PANEL: i32 = 30;
    const TH: i32 = 22;
    const BD: i32 = 1;
    fn snap(z: SnapZone) -> Rect {
        snap_rect(z, FB_W, FB_H, PANEL, TH, BD, 8, 16, 160, 120)
    }
    // The outer-box bottom edge of a content rect (content bottom + border).
    fn outer_bottom(r: Rect) -> i32 {
        r.y + r.h + BD
    }
    fn outer_right(r: Rect) -> i32 {
        r.x + r.w + BD
    }

    #[test]
    fn snap_maximize_fills_work_area_above_the_panel() {
        let r = snap(SnapZone::Maximize);
        // Top-left of the OUTER box hugs the screen corner.
        assert_eq!(r.x - BD, 0);
        assert_eq!(r.y - TH - BD, 0);
        // Cell-snapped content, never under the 30px taskbar (work-area bottom = 800-30 = 770).
        assert_eq!(r.w, snap_cells(1280 - 2, 8)); // 1278 → 1272
        assert_eq!(r.h, snap_cells(770 - 24, 16)); // 746 → 736
        assert!(outer_bottom(r) <= FB_H - PANEL, "maximized window must clear the panel");
        assert_eq!(r.w % 8, 0);
        assert_eq!(r.h % 16, 0);
    }

    #[test]
    fn snap_halves_split_the_work_area() {
        let l = snap(SnapZone::LeftHalf);
        let rt = snap(SnapZone::RightHalf);
        // Left half starts at the left edge; right half starts at the horizontal midline.
        assert_eq!(l.x - BD, 0);
        assert_eq!(rt.x - BD, 640);
        // Both span the full work-area height and clear the panel.
        assert!(outer_bottom(l) <= FB_H - PANEL);
        assert!(outer_bottom(rt) <= FB_H - PANEL);
        // Neither half spills past the right edge, and they don't overlap (left's right ≤ right's x).
        assert!(outer_right(l) <= FB_W);
        assert!(outer_right(rt) <= FB_W);
        assert!(l.x + l.w <= rt.x);
        // Top/bottom halves split vertically at the work-area midline (385).
        let t = snap(SnapZone::TopHalf);
        let b = snap(SnapZone::BottomHalf);
        assert_eq!(t.y - TH - BD, 0);
        assert_eq!(b.y - TH - BD, 385);
        assert!(t.y + t.h <= b.y - TH - BD, "top half must not overlap bottom half's chrome");
        assert!(outer_bottom(b) <= FB_H - PANEL);
    }

    #[test]
    fn snap_quadrants_fit_their_corner_of_the_work_area() {
        for z in [
            SnapZone::TopLeft,
            SnapZone::TopRight,
            SnapZone::BottomLeft,
            SnapZone::BottomRight,
        ] {
            let r = snap(z);
            // Every quadrant stays inside the framebuffer and above the panel, on the cell grid.
            assert!(r.x - BD >= 0 && outer_right(r) <= FB_W, "{z:?} x-bounds");
            assert!(r.y - TH - BD >= 0 && outer_bottom(r) <= FB_H - PANEL, "{z:?} y-bounds");
            assert_eq!(r.w % 8, 0, "{z:?} width on cell grid");
            assert_eq!(r.h % 16, 0, "{z:?} height on cell grid");
        }
        // The right column starts at the midline; the bottom row starts at the vertical midline.
        assert_eq!(snap(SnapZone::TopRight).x - BD, 640);
        assert_eq!(snap(SnapZone::BottomLeft).y - TH - BD, 385);
        assert_eq!(snap(SnapZone::BottomRight).x - BD, 640);
    }

    // ── Drag-to-snap (Aero Snap) ─────────────────────────────────────────────────────────────────

    // Edge band 16px, corner band 160px, on the 1280×800/30-panel screen (work area 1280×770).
    const EDGE: i32 = 16;
    const CORNER: i32 = 160;
    fn esz(mx: i32, my: i32) -> Option<SnapZone> {
        edge_snap_zone(mx, my, FB_W, FB_H, PANEL, EDGE, CORNER)
    }

    #[test]
    fn edge_snap_centre_is_none() {
        // Well inside the screen → no snap (the window drops where dragged).
        assert_eq!(esz(640, 400), None);
        // Just outside every band.
        assert_eq!(esz(EDGE, 400), None); // mx == edge is NOT near_left (strict <).
        assert_eq!(esz(FB_W - EDGE - 1, 400), None);
    }

    #[test]
    fn edge_snap_sides_give_halves() {
        assert_eq!(esz(0, 400), Some(SnapZone::LeftHalf));
        assert_eq!(esz(EDGE - 1, 385), Some(SnapZone::LeftHalf));
        assert_eq!(esz(FB_W - 1, 400), Some(SnapZone::RightHalf));
        assert_eq!(esz(FB_W - EDGE, 385), Some(SnapZone::RightHalf));
    }

    #[test]
    fn edge_snap_top_gives_maximize() {
        // Top band, not in a side band → maximize.
        assert_eq!(esz(640, 0), Some(SnapZone::Maximize));
        assert_eq!(esz(640, EDGE - 1), Some(SnapZone::Maximize));
        // One row below the band is no longer a snap.
        assert_eq!(esz(640, EDGE), None);
    }

    #[test]
    fn edge_snap_corners_give_quadrants() {
        // Top corners: in the side band AND the top corner band.
        assert_eq!(esz(0, 0), Some(SnapZone::TopLeft));
        assert_eq!(esz(0, CORNER - 1), Some(SnapZone::TopLeft));
        assert_eq!(esz(FB_W - 1, 5), Some(SnapZone::TopRight));
        // Bottom corners: side band, just above the panel (work-area bottom = 770).
        let work_h = FB_H - PANEL; // 770
        assert_eq!(esz(0, work_h - 1), Some(SnapZone::BottomLeft));
        assert_eq!(esz(0, work_h - CORNER), Some(SnapZone::BottomLeft));
        assert_eq!(esz(FB_W - 1, work_h - 1), Some(SnapZone::BottomRight));
        // Mid-height on the side stays a half (between the corner bands).
        assert_eq!(esz(0, work_h / 2), Some(SnapZone::LeftHalf));
    }

    #[test]
    fn snap_zone_outer_matches_snap_rect_subdivision() {
        // The preview outer box and the snapped content rect must describe the SAME region: the
        // content rect is the outer box inset by the chrome (border + title bar).
        let work_h = FB_H - PANEL;
        let max = snap_zone_outer(SnapZone::Maximize, FB_W, FB_H, PANEL);
        assert_eq!(max, Rect { x: 0, y: 0, w: FB_W, h: work_h });

        let l = snap_zone_outer(SnapZone::LeftHalf, FB_W, FB_H, PANEL);
        assert_eq!(l, Rect { x: 0, y: 0, w: FB_W / 2, h: work_h });
        let r = snap_zone_outer(SnapZone::RightHalf, FB_W, FB_H, PANEL);
        assert_eq!(r, Rect { x: FB_W / 2, y: 0, w: FB_W - FB_W / 2, h: work_h });
        // The two halves tile the width with no gap or overlap.
        assert_eq!(l.x + l.w, r.x);
        assert_eq!(r.x + r.w, FB_W);

        // A quadrant's outer box and snap_rect's content rect agree once the chrome is added back:
        // content.x == outer.x + border, content.y == outer.y + title_h + border.
        let q_outer = snap_zone_outer(SnapZone::BottomRight, FB_W, FB_H, PANEL);
        let q_content = snap(SnapZone::BottomRight);
        assert_eq!(q_content.x, q_outer.x + BD);
        assert_eq!(q_content.y, q_outer.y + TH + BD);
    }

    #[test]
    fn snap_degenerate_panel_does_not_panic_or_invert() {
        // A panel taller than the screen can't be honoured; the work area floors at one chrome+cell
        // box rather than going negative, so the rect stays valid (w/h ≥ min, on the grid).
        let r = snap_rect(SnapZone::Maximize, 1280, 40, 200, 22, 1, 8, 16, 160, 120);
        assert!(r.w >= 160 && r.h >= 112);
        assert_eq!(r.w % 8, 0);
        assert_eq!(r.h % 16, 0);
    }

    // ── Double-click + focus cycling (C2) ─────────────────────────────────────────────────────────

    #[test]
    fn double_click_needs_same_window_within_window() {
        // No prior press is never a double.
        assert!(!is_double_click(None, 3, 100, 400));
        // Same window, inside the window → double (boundary inclusive).
        assert!(is_double_click(Some((100, 3)), 3, 300, 400));
        assert!(is_double_click(Some((100, 3)), 3, 500, 400)); // exactly +400 ms still counts.
        // Same window but too slow → not a double.
        assert!(!is_double_click(Some((100, 3)), 3, 501, 400));
        // A press on a DIFFERENT window resets the pairing, even if quick.
        assert!(!is_double_click(Some((100, 3)), 2, 200, 400));
        // A non-monotonic clock (now < last) saturates to a 0 gap rather than panicking/underflowing.
        assert!(is_double_click(Some((500, 3)), 3, 100, 400));
    }

    #[test]
    fn focus_cycle_steps_forward_and_wraps() {
        // Four contiguous focusable slots [0,3].
        let all = |i: usize| i < 4;
        assert_eq!(next_focus(Some(0), 4, false, all), Some(1));
        assert_eq!(next_focus(Some(2), 4, false, all), Some(3));
        assert_eq!(next_focus(Some(3), 4, false, all), Some(0)); // wrap.
        // No current focus → forward begins at slot 0.
        assert_eq!(next_focus(None, 4, false, all), Some(0));
    }

    #[test]
    fn focus_cycle_steps_reverse_and_wraps() {
        let all = |i: usize| i < 4;
        assert_eq!(next_focus(Some(2), 4, true, all), Some(1));
        assert_eq!(next_focus(Some(0), 4, true, all), Some(3)); // wrap to the top.
        // No current focus → reverse begins at the last slot.
        assert_eq!(next_focus(None, 4, true, all), Some(3));
    }

    #[test]
    fn focus_cycle_skips_unfocusable_slots() {
        // Only slots 1, 4 and 9 hold focusable z1 windows in a 16-slot table (the rest empty/desktop).
        let mask = |i: usize| matches!(i, 1 | 4 | 9);
        assert_eq!(next_focus(Some(1), 16, false, mask), Some(4));
        assert_eq!(next_focus(Some(9), 16, false, mask), Some(1)); // wrap past the gap.
        assert_eq!(next_focus(Some(4), 16, true, mask), Some(1));
        assert_eq!(next_focus(Some(1), 16, true, mask), Some(9)); // reverse-wrap.
    }

    #[test]
    fn focus_cycle_lone_window_stays_and_empty_is_none() {
        // A single focusable window: cycling lands back on it (covers the whole ring, finds only it).
        let lone = |i: usize| i == 5;
        assert_eq!(next_focus(Some(5), 16, false, lone), Some(5));
        assert_eq!(next_focus(Some(5), 16, true, lone), Some(5));
        // Nothing focusable → None (focus unchanged by the caller).
        assert_eq!(next_focus(Some(5), 16, false, |_| false), None);
        assert_eq!(next_focus(None, 0, false, |_| true), None); // empty ring.
    }

    #[test]
    fn chip_action_follows_the_taskbar_toggle_policy() {
        // Minimized always restores, regardless of the (irrelevant) focus bit.
        assert_eq!(chip_action(true, false), ChipAction::Restore);
        assert_eq!(chip_action(true, true), ChipAction::Restore);
        // Visible + focused → the active-button toggle hides it.
        assert_eq!(chip_action(false, true), ChipAction::Minimize);
        // Visible + unfocused → focus + raise.
        assert_eq!(chip_action(false, false), ChipAction::Focus);
    }
}
