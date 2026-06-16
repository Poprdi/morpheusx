//! Window-management geometry — hit-test, move/resize/snap math, and cursor shape for compd.
//! Pure spatial logic only; `no_std` for compd, links `std` for `cfg(test)`.
//! Tests live here because headless QEMU has no working pointer device (PS/2 mouse is a kernel
//! stub; xHCI init fails), so the pixel math must be pinned by host-runnable unit tests.
#![cfg_attr(not(test), no_std)]

/// Rectangle in framebuffer pixels. For a window: the *content* rect; title bar + border sit outside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Chrome metrics. Close cell: `[right - close_off, right - close_off + close_w)`.
/// Maximize/minimize step left by `btn_pitch` each, same width.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub title_h: i32,
    pub border: i32,
    pub grip: i32,
    pub close_off: i32,
    pub close_w: i32,
    /// Leftward step from one title-bar button's left edge to the next (close → maximize → minimize).
    pub btn_pitch: i32,
}

/// Hit region. Test priority: Close > Maximize > Minimize > Resize > Title > Content.
/// Grip overlaps the content corner but classifies as Resize.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    Title,
    Close,
    Minimize,
    Maximize,
    Resize,
    Content,
}

/// In-progress drag kind; holds cursor shape stable for the whole drag even if the pointer leaves the handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capture {
    Move,
    Resize,
}

/// Cursor glyph: reflects what a press at the current hover would do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorShape {
    Arrow,
    Move,
    Resize,
}

/// Classify a point against a window's chrome; `None` if it misses the outer box or lands on bare border pixels.
/// Test order: outer-box → title buttons → grip → title bar → content.
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

    if my >= tb_y && my < tb_y + c.title_h {
        let close_x = tb_x + tb_w - c.close_off;
        let max_x = close_x - c.btn_pitch;
        let min_x = close_x - 2 * c.btn_pitch;
        if mx >= close_x && mx < close_x + c.close_w {
            return Some(Region::Close);
        }
        if mx >= max_x && mx < max_x + c.close_w {
            return Some(Region::Maximize);
        }
        if mx >= min_x && mx < min_x + c.close_w {
            return Some(Region::Minimize);
        }
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

/// Cursor shape: active drag wins over hover; title/move → Move; grip/resize → Resize; else Arrow.
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

/// Clamped top-left for a title-bar drag: `x ∈ [0, fb_w-w]`, `y ∈ [title_h, fb_h-h]`.
pub fn clamp_move(w: i32, h: i32, fb_w: i32, fb_h: i32, title_h: i32, nx: i32, ny: i32) -> (i32, i32) {
    let max_x = (fb_w - w).max(0);
    let max_y = (fb_h - h).max(title_h);
    (nx.clamp(0, max_x), ny.clamp(title_h, max_y))
}

/// Round pixel extent DOWN to whole cells; 1:1 blit leaves a stale strip on fractional edges.
#[inline]
pub fn snap_cells(px: i32, cell: i32) -> i32 {
    let cell = cell.max(1);
    (px.max(0) / cell) * cell
}

/// New `(w, h)` from a grip drag: `start + delta`, clamped to `[min, fb - origin]`, snapped DOWN to cells.
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
// Ctrl+Alt chord table + geometry step math. Both live here so keyboard-WM behaviour is
// host-tested: chord table → command, command → clamp/snap math, all without a boot pointer.
// PS/2 Set-1 scancodes for the vim direction cluster + close. compd passes the release-edge make.
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

/// Move/resize step: 32px = 4×8px columns / 2×16px rows. Resize deltas are cell-snapped.
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

/// Tiling target for the focused window. Nine zones map like a numpad (corners=quadrants,
/// edges=halves, centre=maximize toggle). All zones fill the work area above the panel.
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

/// Keyboard WM command: Move nudges, Resize grows an edge, Snap tiles to a work-area zone, Close terminates.
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

/// Map scancode + modifiers to a WM command, or `None` to forward to the client. Namespace is
/// Ctrl+Alt: bare HJKL moves, +Shift resizes, C closes. AltGr is tracked separately, so composing chords
/// never collide.
pub fn wm_command(scancode: u8, ctrl: bool, alt: bool, shift: bool) -> Option<WmCommand> {
    if !(ctrl && alt) {
        return None;
    }
    // Snap on number-row grid; Shift-independent (digits don't collide with HJKL/C).
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

/// Apply Move/Resize to the content rect using the same clamp/snap as mouse drag; Close is a no-op.
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
        // Snap needs panel_h, border, and saved-rect state; caller resolves via snap_rect.
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

/// Content rect for a window snapped to `zone`, inset from the zone's outer box by chrome
/// (title bar + border). Width/height snapped DOWN to whole cells; floored at `min_w`/`min_h`.
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
    // Never let the work area collapse below one chrome+cell box.
    let wa_h = (fb_h - panel_h).max(title_h + border * 2 + cell_h);

    let (ox, oy, ow, oh) = zone_outer_box(zone, wa_w, wa_h);

    let x = ox + border;
    let y = oy + title_h + border;
    let w = snap_cells((ow - border * 2).max(min_w), cell_w);
    let h = snap_cells((oh - (title_h + border * 2)).max(min_h), cell_h);
    Rect { x, y, w, h }
}

/// Chrome+content outer box for `zone` in a `wa_w×wa_h` work area, before chrome inset.
/// Shared by [`snap_rect`] and [`snap_zone_outer`] so both describe the same region.
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

/// Full chrome+content outer box for `zone` in framebuffer pixels — what the drag-to-snap preview
/// highlights. Same box [`snap_rect`] insets; the preview must not use the content rect.
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

/// True if `last` press was on the same window and within `threshold_ms`. `None` history is never
/// a double. `saturating_sub` guards against a non-monotonic clock.
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

/// Next focusable slot in a ring of `n` from `current`, stepping `+1` (forward) or `-1` (reverse).
/// Returns `None` only if nothing is focusable. No-focus forward → starts at 0; reverse → `n-1`.
pub fn next_focus(
    current: Option<usize>,
    n: usize,
    reverse: bool,
    focusable: impl Fn(usize) -> bool,
) -> Option<usize> {
    if n == 0 {
        return None;
    }
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

/// What activating a taskbar chip does: minimized → restore; focused-and-visible → minimize (toggle);
/// otherwise → focus+raise. Canonical desktop policy; pure so the toggle is host-tested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipAction {
    /// Un-minimize the window and give it focus (it was hidden).
    Restore,
    /// Hide the window — it is the currently-focused, visible one (the active-button toggle).
    Minimize,
    /// Bring an already-visible, unfocused window to the front and focus it.
    Focus,
}

/// Map `(is_minimized, is_focused)` to the taskbar-chip action per [`ChipAction`] policy.
pub fn chip_action(is_minimized: bool, is_focused: bool) -> ChipAction {
    if is_minimized {
        ChipAction::Restore
    } else if is_focused {
        ChipAction::Minimize
    } else {
        ChipAction::Focus
    }
}

// Overview (Exposé) — thumbnail grid layout and hit-test.
// compd owns mode, surface scaling, and focus edit; this crate owns only the geometry.

/// Thumbnail placement: `cell` is the full clickable tile; `thumb` is the aspect-fitted blit rect inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OverviewSlot {
    pub cell: Rect,
    pub thumb: Rect,
}

/// Grid dimensions `(cols, rows)` for `n` thumbnails: `cols = ⌈√n⌉`, `rows = ⌈n/cols⌉`. Integer-only (`no_std`).
pub fn overview_dims(n: u32) -> (u32, u32) {
    if n == 0 {
        return (0, 0);
    }
    // Smallest c with c*c >= n.
    let mut cols = 1u32;
    while cols * cols < n {
        cols += 1;
    }
    let rows = n.div_ceil(cols);
    (cols, rows)
}

/// The `(col, row)` of thumbnail `i` in a row-major grid of `cols` columns.
fn overview_cell_rc(i: u32, cols: u32) -> (u32, u32) {
    let cols = cols.max(1);
    (i % cols, i / cols)
}

/// Full grid-cell rect for thumbnail `i` of `n`, equal-sized row-major, `margin`+`gap` layout.
/// Zeroed rect for out-of-range `i`.
pub fn overview_cell(i: u32, n: u32, area: Rect, margin: i32, gap: i32) -> Rect {
    let (cols, rows) = overview_dims(n);
    if i >= n || cols == 0 || rows == 0 {
        return Rect { x: 0, y: 0, w: 0, h: 0 };
    }
    let inner_w = (area.w - 2 * margin - gap * (cols as i32 - 1)).max(1);
    let inner_h = (area.h - 2 * margin - gap * (rows as i32 - 1)).max(1);
    let cw = (inner_w / cols as i32).max(1);
    let ch = (inner_h / rows as i32).max(1);
    let (c, r) = overview_cell_rc(i, cols);
    Rect {
        x: area.x + margin + c as i32 * (cw + gap),
        y: area.y + margin + r as i32 * (ch + gap),
        w: cw,
        h: ch,
    }
}

/// `win_w×win_h` aspect-fitted into `cell` minus `pad`+`label_h`, centered. Never upscaled past 1:1.
pub fn overview_thumb(cell: Rect, win_w: i32, win_h: i32, pad: i32, label_h: i32) -> Rect {
    let box_x = cell.x + pad;
    let box_y = cell.y + pad;
    let box_w = (cell.w - 2 * pad).max(1);
    let box_h = (cell.h - 2 * pad - label_h).max(1);
    if win_w <= 0 || win_h <= 0 {
        return Rect { x: box_x, y: box_y, w: box_w, h: box_h };
    }
    // scale = min(box_w/win_w, box_h/win_h, 1) in fixed-point (×65536).
    let sx = ((box_w as i64) << 16) / win_w as i64;
    let sy = ((box_h as i64) << 16) / win_h as i64;
    let scale = sx.min(sy).min(1 << 16);
    let tw = (((win_w as i64) * scale) >> 16).clamp(1, box_w as i64) as i32;
    let th = (((win_h as i64) * scale) >> 16).clamp(1, box_h as i64) as i32;
    Rect {
        x: box_x + (box_w - tw) / 2,
        y: box_y + (box_h - th) / 2,
        w: tw,
        h: th,
    }
}

/// Hit-test a pointer against the overview grid; whole cell is clickable, not just the thumbnail.
/// Returns index or `None` if the point misses all cells.
pub fn overview_hit(n: u32, area: Rect, margin: i32, gap: i32, mx: i32, my: i32) -> Option<u32> {
    for i in 0..n {
        let cell = overview_cell(i, n, area, margin, gap);
        if mx >= cell.x && mx < cell.x + cell.w && my >= cell.y && my < cell.y + cell.h {
            return Some(i);
        }
    }
    None
}

/// Step overview selection in `dir`, row-major. Left/Right clamp at row ends; Up/Down step a row.
pub fn overview_nav(sel: u32, n: u32, dir: Dir) -> u32 {
    if n == 0 {
        return sel;
    }
    let (cols, _rows) = overview_dims(n);
    let cols = cols.max(1);
    let sel = sel.min(n - 1);
    match dir {
        Dir::Left => {
            if sel % cols == 0 {
                sel
            } else {
                sel - 1
            }
        },
        Dir::Right => {
            if sel % cols == cols - 1 || sel + 1 >= n {
                sel
            } else {
                sel + 1
            }
        },
        Dir::Up => {
            if sel < cols {
                sel
            } else {
                sel - cols
            }
        },
        Dir::Down => {
            let down = sel + cols;
            if down < n {
                down
            } else {
                sel
            }
        },
    }
}

// Window context menu — popup layout, row hit-test, and menu→snap mapping.
// compd owns lifetime and drawing; each row maps to an edit the keyboard/buttons already have.

/// Action a window-menu row performs; see [`menu_action_zone`] for snap mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuAction {
    MaximizeRestore,
    Minimize,
    SnapLeft,
    SnapRight,
    SnapTop,
    SnapBottom,
    Close,
}

/// One row of the window menu: a chooseable item or an inert separator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    Item { action: MenuAction, label: &'static str },
    Separator,
}

/// Popup pixel metrics: row/separator heights, text padding, font cell width, minimum box width.
#[derive(Clone, Copy)]
pub struct MenuMetrics {
    pub row_h: i32,
    pub sep_h: i32,
    pub pad_x: i32,
    pub char_w: i32,
    pub min_w: i32,
}

/// Fixed row count of [`window_menu`]; lets compd hold the array without alloc.
pub const WINDOW_MENU_ROWS: usize = 9;

/// Window menu rows in display order. First row toggles Maximize/Restore; two separators group ops · snaps · close.
pub fn window_menu(maximized: bool) -> [MenuRow; WINDOW_MENU_ROWS] {
    [
        MenuRow::Item {
            action: MenuAction::MaximizeRestore,
            label: if maximized { "Restore" } else { "Maximize" },
        },
        MenuRow::Item { action: MenuAction::Minimize, label: "Minimize" },
        MenuRow::Separator,
        MenuRow::Item { action: MenuAction::SnapLeft, label: "Snap left" },
        MenuRow::Item { action: MenuAction::SnapRight, label: "Snap right" },
        MenuRow::Item { action: MenuAction::SnapTop, label: "Snap top" },
        MenuRow::Item { action: MenuAction::SnapBottom, label: "Snap bottom" },
        MenuRow::Separator,
        MenuRow::Item { action: MenuAction::Close, label: "Close" },
    ]
}

/// Popup size: width = widest label × `char_w` + `2*pad_x` (floored at `min_w`); height = sum of row heights.
pub fn menu_size(rows: &[MenuRow], m: MenuMetrics) -> (i32, i32) {
    let mut max_chars = 0i32;
    let mut h = 0i32;
    for row in rows {
        match row {
            MenuRow::Item { label, .. } => {
                max_chars = max_chars.max(label.chars().count() as i32);
                h += m.row_h;
            },
            MenuRow::Separator => h += m.sep_h,
        }
    }
    let w = (max_chars * m.char_w + 2 * m.pad_x).max(m.min_w);
    (w, h)
}

/// Top-left for a `w×h` menu at pointer `(ax, ay)`, clamped so the box stays inside the work area.
pub fn menu_origin(
    ax: i32,
    ay: i32,
    w: i32,
    h: i32,
    fb_w: i32,
    fb_h: i32,
    panel_h: i32,
) -> (i32, i32) {
    let work_h = (fb_h - panel_h).max(1);
    let x = ax.min((fb_w - w).max(0)).max(0);
    let y = ay.min((work_h - h).max(0)).max(0);
    (x, y)
}

/// Full-width rect of row `i` in a menu at `(ox, oy)` with width `w`. Zero rect for out-of-range `i`.
pub fn menu_row_rect(rows: &[MenuRow], m: MenuMetrics, ox: i32, oy: i32, w: i32, i: usize) -> Rect {
    let mut y = oy;
    for (j, row) in rows.iter().enumerate() {
        let rh = match row {
            MenuRow::Separator => m.sep_h,
            _ => m.row_h,
        };
        if j == i {
            return Rect { x: ox, y, w, h: rh };
        }
        y += rh;
    }
    Rect { x: 0, y: 0, w: 0, h: 0 }
}

/// Hit-test pointer against menu → item row index, or `None` if it misses or hits a separator.
pub fn menu_hit(
    rows: &[MenuRow],
    m: MenuMetrics,
    ox: i32,
    oy: i32,
    w: i32,
    mx: i32,
    my: i32,
) -> Option<usize> {
    if mx < ox || mx >= ox + w {
        return None;
    }
    let mut y = oy;
    for (i, row) in rows.iter().enumerate() {
        let rh = match row {
            MenuRow::Separator => m.sep_h,
            _ => m.row_h,
        };
        if my >= y && my < y + rh {
            return match row {
                MenuRow::Item { .. } => Some(i),
                MenuRow::Separator => None,
            };
        }
        y += rh;
    }
    None
}

/// Next selectable row from `sel` stepping up/down, skipping separators, wrapping at ends.
pub fn menu_nav(sel: usize, rows: &[MenuRow], down: bool) -> usize {
    let n = rows.len();
    if n == 0 {
        return sel;
    }
    let mut i = sel.min(n - 1);
    for _ in 0..n {
        i = if down {
            if i + 1 >= n {
                0
            } else {
                i + 1
            }
        } else if i == 0 {
            n - 1
        } else {
            i - 1
        };
        if matches!(rows[i], MenuRow::Item { .. }) {
            return i;
        }
    }
    sel
}

/// Snap zone for a menu action, or `None` for `Minimize`/`Close` (handled directly by compd).
pub fn menu_action_zone(a: MenuAction) -> Option<SnapZone> {
    Some(match a {
        MenuAction::MaximizeRestore => SnapZone::Maximize,
        MenuAction::SnapLeft => SnapZone::LeftHalf,
        MenuAction::SnapRight => SnapZone::RightHalf,
        MenuAction::SnapTop => SnapZone::TopHalf,
        MenuAction::SnapBottom => SnapZone::BottomHalf,
        MenuAction::Minimize | MenuAction::Close => return None,
    })
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
        btn_pitch: 32,
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
    fn maximize_and_minimize_buttons_classify_left_of_close() {
        let right = WIN.x + WIN.w;
        let cy = WIN.y - C.title_h / 2;
        let close_x = right - C.close_off;
        // Centre of the maximize cell (one pitch left of close).
        let max_cx = close_x - C.btn_pitch + C.close_w / 2;
        assert_eq!(classify(WIN, C, max_cx, cy), Some(Region::Maximize));
        // Centre of the minimize cell (two pitches left of close).
        let min_cx = close_x - 2 * C.btn_pitch + C.close_w / 2;
        assert_eq!(classify(WIN, C, min_cx, cy), Some(Region::Minimize));
        // The three button cells are distinct and ordered min < max < close.
        assert!(min_cx < max_cx && max_cx < close_x + C.close_w / 2);
        // Just left of the minimize cell is plain title bar (the buttons don't bleed into the title).
        assert_eq!(
            classify(WIN, C, close_x - 2 * C.btn_pitch - 1, cy),
            Some(Region::Title)
        );
        // The 2px gap between maximize's right edge and close's left edge is title, not a button.
        assert_eq!(classify(WIN, C, close_x - 1, cy), Some(Region::Title));
    }

    #[test]
    fn title_buttons_only_within_the_title_band() {
        // A column that would be a button, but a row inside the content area, is content — the
        // buttons are clamped to the title-bar y-band.
        let right = WIN.x + WIN.w;
        let max_cx = right - C.close_off - C.btn_pitch + C.close_w / 2;
        assert_eq!(classify(WIN, C, max_cx, WIN.y + 10), Some(Region::Content));
    }

    #[test]
    fn buttons_keep_the_arrow_cursor() {
        // A press on a control button is a click, not a drag — the pointer stays an arrow over them
        // (only the title→move and grip→resize affordances change the cursor).
        assert_eq!(cursor_shape(None, Some(Region::Minimize)), CursorShape::Arrow);
        assert_eq!(cursor_shape(None, Some(Region::Maximize)), CursorShape::Arrow);
        assert_eq!(cursor_shape(None, Some(Region::Close)), CursorShape::Arrow);
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

    // ---- overview (Exposé) grid ------------------------------------------------

    #[test]
    fn overview_dims_is_near_square_ceil_sqrt() {
        assert_eq!(overview_dims(0), (0, 0));
        assert_eq!(overview_dims(1), (1, 1));
        assert_eq!(overview_dims(2), (2, 1)); // ceil(sqrt 2)=2 cols, 1 row
        assert_eq!(overview_dims(3), (2, 2));
        assert_eq!(overview_dims(4), (2, 2));
        assert_eq!(overview_dims(5), (3, 2));
        assert_eq!(overview_dims(9), (3, 3));
        assert_eq!(overview_dims(10), (4, 3));
        assert_eq!(overview_dims(16), (4, 4));
    }

    #[test]
    fn overview_cells_tile_the_area_left_to_right_top_to_bottom() {
        let area = Rect { x: 0, y: 0, w: 1280, h: 770 };
        // 4 windows → 2×2. Equal cells, margin 20, gap 16.
        let (m, g) = (20, 16);
        let c0 = overview_cell(0, 4, area, m, g);
        let c1 = overview_cell(1, 4, area, m, g);
        let c2 = overview_cell(2, 4, area, m, g);
        let c3 = overview_cell(3, 4, area, m, g);
        // top-left starts at the margin.
        assert_eq!((c0.x, c0.y), (20, 20));
        // identical cell sizes.
        assert_eq!((c0.w, c0.h), (c1.w, c1.h));
        assert_eq!((c0.w, c0.h), (c3.w, c3.h));
        // col 1 is one (cell+gap) to the right of col 0; row 1 one down.
        assert_eq!(c1.x, c0.x + c0.w + g);
        assert_eq!(c1.y, c0.y);
        assert_eq!(c2.x, c0.x);
        assert_eq!(c2.y, c0.y + c0.h + g);
        // the grid fits inside the area.
        assert!(c3.x + c3.w <= area.w - m + 1);
        assert!(c3.y + c3.h <= area.h - m + 1);
        // out-of-range is a zero rect.
        assert_eq!(overview_cell(4, 4, area, m, g), Rect { x: 0, y: 0, w: 0, h: 0 });
    }

    #[test]
    fn overview_thumb_preserves_aspect_and_never_upscales() {
        let cell = Rect { x: 100, y: 100, w: 400, h: 300 };
        // A wide 1280×800 window into the cell (pad 8, label 18): width-bound, aspect held.
        let t = overview_thumb(cell, 1280, 800, 8, 18);
        // aspect ~1.6 preserved within rounding.
        let ar_win = 1280f64 / 800f64;
        let ar_thumb = t.w as f64 / t.h as f64;
        assert!((ar_win - ar_thumb).abs() < 0.05, "aspect drift: {ar_thumb}");
        // fits inside the padded box.
        assert!(t.x >= cell.x + 8 && t.y >= cell.y + 8);
        assert!(t.x + t.w <= cell.x + cell.w - 8);
        assert!(t.y + t.h <= cell.y + cell.h - 8 - 18);
        // centered horizontally (width-bound case fills width; centering shows on the y axis).
        // A tiny window is shown at native size, not magnified.
        let small = overview_thumb(cell, 40, 30, 8, 18);
        assert_eq!((small.w, small.h), (40, 30));
        // degenerate dims fall back to filling the box.
        let deg = overview_thumb(cell, 0, 0, 8, 18);
        assert_eq!((deg.w, deg.h), (400 - 16, 300 - 16 - 18));
    }

    #[test]
    fn overview_hit_maps_points_to_their_cell() {
        let area = Rect { x: 0, y: 0, w: 1280, h: 770 };
        let (m, g) = (20, 16);
        // center of each cell hits its index.
        for i in 0..4u32 {
            let c = overview_cell(i, 4, area, m, g);
            assert_eq!(overview_hit(4, area, m, g, c.x + c.w / 2, c.y + c.h / 2), Some(i));
        }
        // a point in the outer margin misses.
        assert_eq!(overview_hit(4, area, m, g, 2, 2), None);
        // a point in the inter-cell gap misses.
        let c0 = overview_cell(0, 4, area, m, g);
        assert_eq!(overview_hit(4, area, m, g, c0.x + c0.w + g / 2, c0.y + 5), None);
    }

    #[test]
    fn overview_nav_steps_the_grid_and_clamps() {
        // 5 windows → 3 cols × 2 rows; indices 0..4 (last row partial: 3,4 only).
        let n = 5;
        // right within a row, clamp at the row end.
        assert_eq!(overview_nav(0, n, Dir::Right), 1);
        assert_eq!(overview_nav(2, n, Dir::Right), 2); // end of row 0, no wrap
        // left clamps at col 0.
        assert_eq!(overview_nav(0, n, Dir::Left), 0);
        assert_eq!(overview_nav(1, n, Dir::Left), 0);
        // down a row.
        assert_eq!(overview_nav(0, n, Dir::Down), 3);
        // down off the partial last row stays put (index 2 has no cell below: 2+3=5 ≥ n).
        assert_eq!(overview_nav(2, n, Dir::Down), 2);
        // up a row.
        assert_eq!(overview_nav(3, n, Dir::Up), 0);
        assert_eq!(overview_nav(0, n, Dir::Up), 0);
        // empty grid: unchanged.
        assert_eq!(overview_nav(0, 0, Dir::Right), 0);
    }

    // ── Window context menu ──────────────────────────────────────────────────────────────────

    const MM: MenuMetrics = MenuMetrics {
        row_h: 20,
        sep_h: 7,
        pad_x: 12,
        char_w: 8,
        min_w: 120,
    };

    #[test]
    fn window_menu_toggles_first_label_and_has_two_separators() {
        let m = window_menu(false);
        assert_eq!(m.len(), WINDOW_MENU_ROWS);
        assert_eq!(
            m[0],
            MenuRow::Item { action: MenuAction::MaximizeRestore, label: "Maximize" }
        );
        // Maximized → the first row offers Restore instead.
        assert_eq!(
            window_menu(true)[0],
            MenuRow::Item { action: MenuAction::MaximizeRestore, label: "Restore" }
        );
        let seps = m.iter().filter(|r| matches!(r, MenuRow::Separator)).count();
        assert_eq!(seps, 2);
        assert_eq!(m[WINDOW_MENU_ROWS - 1], MenuRow::Item { action: MenuAction::Close, label: "Close" });
    }

    #[test]
    fn menu_size_fits_widest_label_and_sums_row_heights() {
        let rows = window_menu(false);
        let (_w, h) = menu_size(&rows, MM);
        // 7 item rows + 2 separators.
        assert_eq!(h, 7 * MM.row_h + 2 * MM.sep_h);
        // Widest-label path: a low floor lets the widest label ("Snap bottom", 11 chars) size the box.
        let wide = MenuMetrics { min_w: 0, ..MM };
        assert_eq!(menu_size(&rows, wide).0, 11 * MM.char_w + 2 * MM.pad_x);
        // Floor path: a sliver of a menu is held at min_w.
        let tiny = [MenuRow::Item { action: MenuAction::Close, label: "X" }];
        assert_eq!(menu_size(&tiny, MM).0, MM.min_w);
    }

    #[test]
    fn menu_origin_prefers_anchor_then_clamps_into_the_work_area() {
        let (fb_w, fb_h, panel) = (1280, 800, 30);
        let (w, h) = (160, 220);
        // Comfortably inside → dropped exactly at the cursor.
        assert_eq!(menu_origin(400, 300, w, h, fb_w, fb_h, panel), (400, 300));
        // Near the right edge → pulled left so the far edge is flush at fb_w.
        assert_eq!(menu_origin(1200, 300, w, h, fb_w, fb_h, panel).0, fb_w - w);
        // Near the bottom → pulled up so the box clears the panel (work area = fb_h - panel).
        assert_eq!(menu_origin(400, 790, w, h, fb_w, fb_h, panel).1, (fb_h - panel) - h);
        // A negative/odd anchor never escapes the top-left.
        assert_eq!(menu_origin(-50, -50, w, h, fb_w, fb_h, panel), (0, 0));
    }

    #[test]
    fn menu_hit_finds_items_and_ignores_separators_and_the_outside() {
        let rows = window_menu(false);
        let (w, _h) = menu_size(&rows, MM);
        let (ox, oy) = (100, 100);
        // Row 0 (Maximize): its band is [oy, oy+row_h).
        let r0 = menu_row_rect(&rows, MM, ox, oy, w, 0);
        assert_eq!(r0, Rect { x: ox, y: oy, w, h: MM.row_h });
        assert_eq!(menu_hit(&rows, MM, ox, oy, w, ox + 5, oy + 3), Some(0));
        // Row 1 (Minimize).
        assert_eq!(menu_hit(&rows, MM, ox, oy, w, ox + 5, oy + MM.row_h + 3), Some(1));
        // Row 2 is a separator → inert.
        let sep_y = oy + 2 * MM.row_h + MM.sep_h / 2;
        assert_eq!(menu_hit(&rows, MM, ox, oy, w, ox + 5, sep_y), None);
        // Just left of the box → miss.
        assert_eq!(menu_hit(&rows, MM, ox, oy, w, ox - 1, oy + 3), None);
        // Below the box → miss.
        let (_w, h) = menu_size(&rows, MM);
        assert_eq!(menu_hit(&rows, MM, ox, oy, w, ox + 5, oy + h), None);
    }

    #[test]
    fn menu_nav_skips_separators_and_wraps() {
        let rows = window_menu(false);
        // Down from Minimize (idx 1) skips the separator at 2 → Snap left (idx 3).
        assert_eq!(menu_nav(1, &rows, true), 3);
        // Down from the last item (Close, idx 8) wraps to the first item (idx 0).
        assert_eq!(menu_nav(8, &rows, true), 0);
        // Up from the first item wraps to the last item.
        assert_eq!(menu_nav(0, &rows, false), 8);
        // Up from Snap left (idx 3) skips the separator → Minimize (idx 1).
        assert_eq!(menu_nav(3, &rows, false), 1);
    }

    #[test]
    fn menu_action_zone_maps_snaps_and_leaves_min_close_alone() {
        assert_eq!(menu_action_zone(MenuAction::MaximizeRestore), Some(SnapZone::Maximize));
        assert_eq!(menu_action_zone(MenuAction::SnapLeft), Some(SnapZone::LeftHalf));
        assert_eq!(menu_action_zone(MenuAction::SnapRight), Some(SnapZone::RightHalf));
        assert_eq!(menu_action_zone(MenuAction::SnapTop), Some(SnapZone::TopHalf));
        assert_eq!(menu_action_zone(MenuAction::SnapBottom), Some(SnapZone::BottomHalf));
        assert_eq!(menu_action_zone(MenuAction::Minimize), None);
        assert_eq!(menu_action_zone(MenuAction::Close), None);
    }
}
