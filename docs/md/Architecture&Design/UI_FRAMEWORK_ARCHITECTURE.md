# MorpheusX UI Framework — Architecture & Invariants

## 0. Document Purpose

This document locks in every architectural decision, invariant, optimization strategy, and constraint for the MorpheusX UI framework before a single line of implementation code is written. It serves as the single source of truth for all implementation work.

---

## 1. Vision

A **standalone, open-sourceable, `no_std` bare-metal UI framework** in Rust that provides:

- A **TTY shell** as the default surface (command prompt, text I/O)
- **Windowed applications** spawned by shell commands (e.g. `open distro-downloader`)
- A **stacking window manager** with damage-tracked compositing
- A **widget toolkit** sufficient for forms, lists, buttons, text areas
- **Zero OS dependencies** — runs on any linear framebuffer with a heap allocator

The mental model: **"tty + x11"** — a text shell is always there, graphical windows float on top when invoked.

---

## 2. Crate Identity

| Field | Value |
|-------|-------|
| Crate name | `morpheus-ui` |
| Location | `ui/` in workspace root |
| `no_std` | Yes, always |
| `alloc` | Required — `Vec`, `Box`, `String` used freely |
| External deps | **None** (standalone) |
| Features | `framebuffer-backend` (default), `software-cursor` |
| Min Rust | 1.75 (workspace standard) |

The crate does **not** depend on `morpheus-display`. Instead, it defines its own `Canvas` trait. The existing `display` crate's `Framebuffer` will implement `Canvas` in the bootloader via a thin adapter, keeping the UI crate fully decoupled.

---

## 3. Layer Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                    Shell / Desktop                       │  Layer 5
│            TTY prompt + command dispatch                 │
├─────────────────────────────────────────────────────────┤
│                   Window Manager                        │  Layer 4
│          Z-order, focus, move, resize, compose          │
├─────────────────────────────────────────────────────────┤
│                    Widget Toolkit                        │  Layer 3
│       Button, Label, TextInput, List, Panel, etc.       │
├─────────────────────────────────────────────────────────┤
│                   Drawing Primitives                    │  Layer 2
│     fill_rect, hline, vline, blit, blend, draw_glyph   │
├─────────────────────────────────────────────────────────┤
│                   Canvas Trait + Color                  │  Layer 1
│      put_pixel, get_pixel, fill_rect, blit, dims        │
└─────────────────────────────────────────────────────────┘
           ▲                              ▲
           │                              │
    Framebuffer adapter           OffscreenBuffer
    (morpheus-display)            (heap Vec<u32>)
```

Each layer depends **only** on the layer directly below it. No layer may reach down more than one level.

---

## 4. Layer 1 — Canvas Trait & Color

### 4.1 Color Type

```text
Color { r: u8, g: u8, b: u8, a: u8 }
```

**Invariants:**
- `Color::rgb(r,g,b)` sets `a = 255` (fully opaque)
- `Color::rgba(r,g,b,a)` preserves alpha
- `to_packed(format) -> u32` encodes per pixel format, alpha written to reserved byte for blending reads
- All blending math uses **integer-only** arithmetic — no floats anywhere in the crate
- Alpha blend formula (Porter-Duff src-over): `out_c = src_c + dst_c * (255 - src_a) / 255`
- Division by 255 approximated as `(x + 128) >> 8` which is exact for all u8 inputs after the multiply

**Pixel format awareness:**
- `PixelFormat` enum: `Bgrx`, `Rgbx` (matches existing display crate)
- `to_packed()` and `from_packed()` handle conversion in both directions
- Every color constant is `const fn`

### 4.2 Canvas Trait

```rust
pub trait Canvas {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn pixel_format(&self) -> PixelFormat;

    fn put_pixel(&mut self, x: u32, y: u32, color: Color);
    fn get_pixel(&self, x: u32, y: u32) -> Color;

    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color);
    fn blit(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32);
    fn blit_blend(&mut self, dst_x: u32, dst_y: u32, src: &[u32], src_w: u32, src_h: u32);

    fn stride(&self) -> u32;
}
```

**Design decisions:**
- `get_pixel` is **required** (not optional) because alpha blending reads the destination
- `fill_rect` is in the trait (not just a provided method) so hardware backends can override with `memset32`
- `blit` is opaque copy (no alpha) — `blit_blend` does src-over compositing
- All methods silently clip to bounds — no panics, no errors
- Coordinates are `u32` throughout (never `i32`, `usize`, or `isize`)

### 4.3 OffscreenBuffer

Heap-allocated `Vec<u32>` buffer implementing `Canvas`:

```text
OffscreenBuffer {
    pixels: Vec<u32>,     // w * h packed pixels
    width: u32,
    height: u32,
    format: PixelFormat,
}
```

**Invariants:**
- `pixels.len() == (width * height) as usize` always
- Stride == width (no padding in offscreen buffers, only hardware framebuffers have stride != width)
- `put_pixel` / `get_pixel` are direct array index: `pixels[(y * width + x) as usize]`
- `fill_rect` fills row-by-row with `slice.fill(value)` — compiler vectorizes this
- `blit` into an OffscreenBuffer uses `copy_from_slice` per row
- `new()` zero-initializes (transparent black)

### 4.4 Framebuffer Adapter (lives in bootloader, not in ui crate)

A thin `impl Canvas for FramebufferCanvas` that wraps `morpheus-display`'s ASM primitives:
- `put_pixel` → `asm_fb_write32`
- `get_pixel` → `asm_fb_read32` (**currently unwired — must be connected**)
- `fill_rect` → row-by-row `asm_fb_memset32`
- `blit` → row-by-row `asm_fb_memcpy`
- `blit_blend` → per-pixel read32/blend/write32 (slow path, used only for translucent windows)

---

## 5. Layer 2 — Drawing Primitives

All primitives operate on any `&mut dyn Canvas`. Stateless free functions grouped in a `draw` module.

### 5.1 Functions

| Function | Strategy |
|----------|----------|
| `hline(canvas, x, y, w, color)` | Single `fill_rect(x, y, w, 1, color)` |
| `vline(canvas, x, y, h, color)` | Single `fill_rect(x, y, 1, h, color)` |
| `rect_outline(canvas, x, y, w, h, thickness, color)` | 4 fill_rects (top, bottom, left, right) |
| `rounded_rect_fill(canvas, x, y, w, h, radius, color)` | Scanline: full-width fill_rect for straight rows, pixel-level for corner arcs |
| `rounded_rect_outline(canvas, x, y, w, h, radius, thickness, color)` | Bresenham circle quadrants + 4 straight edges |
| `circle_fill(canvas, cx, cy, r, color)` | Midpoint circle → horizontal fill_rect per scanline pair |
| `circle_outline(canvas, cx, cy, r, color)` | Midpoint circle, 8-way symmetry, put_pixel |
| `line(canvas, x0, y0, x1, y1, color)` | Bresenham's line, `put_pixel` per point |
| `draw_glyph(canvas, x, y, glyph, fg, bg)` | Row-major: 8 pixels per row via `fill_rect(x, y, 8, 1, ...)` for solid-color runs, or per-pixel |
| `blit_region(canvas, dst_x, dst_y, src, sx, sy, w, h)` | Sub-rect blit from source buffer |

### 5.2 Glyph Rendering Optimization

**Current problem:** `TextConsole` does 128 individual `asm_fb_write32` calls per glyph (8×16 = 128 pixels).

**Optimization — run-length encoding per row:**
For each of the 16 glyph rows, scan the 8-bit pattern and collapse consecutive same-color pixels into a single `fill_rect` call. Worst case (alternating bits) = 8 calls/row = 128 calls/glyph (no worse). Best case (solid row) = 1 call/row = 16 calls/glyph. Average ASCII text: ~3-4 calls/row = 48-64 calls/glyph (**2× speedup**).

**Optimization — batch row writes:**
For fully solid rows (0x00 or 0xFF), emit a single `fill_rect(x, y, 8, 1, color)`. The `0x00` case (all background) is extremely common in the top/bottom padding rows of most glyphs.

### 5.3 Clipping

**All primitives clip to canvas bounds before any pixel writes.** Clipping is computed once per primitive call (bounding box intersection), not per pixel.

```text
Clip rect = intersection(primitive_bbox, canvas_bbox)
If clip rect is empty → return immediately (zero cost)
```

For sub-canvas drawing (widget rendering into a window), a `ClipCanvas` wrapper restricts all operations to a sub-rectangle, enabling per-widget clipping without any per-pixel branch.

---

## 6. Layer 3 — Widget Toolkit

### 6.1 Core Abstractions

```text
Widget trait:
    fn size_hint(&self) -> (u32, u32)     // preferred (w, h)
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme)
    fn handle_event(&mut self, event: &Event) -> EventResult
```

**Invariants:**
- Widgets never know their absolute screen position — they render at (0,0) into a provided canvas
- The parent (layout/window) positions the canvas via `ClipCanvas` or `OffscreenBuffer` offset
- Widgets are **retained mode** — state persists between renders, `render()` is called on damage
- No widget allocates per-frame — all allocations happen at construction

### 6.2 Widget Set (Phase 1)

| Widget | Description |
|--------|-------------|
| `Label` | Static text, single or multi-line, text-color + bg-color |
| `Button` | Label + border, states: normal/focused/pressed, renders focus ring |
| `TextInput` | Single-line editable text with cursor, handles keyboard events |
| `TextArea` | Multi-line scrollable text (for shell output), ring buffer backing |
| `List` | Scrollable list of selectable items, virtual scrolling for large lists |
| `Panel` | Container with optional border and background fill |
| `ProgressBar` | Horizontal fill bar with percentage |
| `Divider` | Horizontal or vertical line |
| `Checkbox` | Toggle with `[X]` / `[ ]` rendering |

### 6.3 Layout

Phase 1: **Manual positioning only.** Each widget placed at explicit (x, y, w, h). This is sufficient for the tty+x11 model where windows have known layouts.

Phase 2 (future): `VStack`, `HStack`, `Grid` layout containers with constraint-based sizing.

### 6.4 Theme

```text
Theme {
    bg: Color,              // window background
    fg: Color,              // default text
    accent: Color,          // focused item border, selected item
    border: Color,          // window border, panel border
    button_bg: Color,
    button_fg: Color,
    button_focus_bg: Color,
    input_bg: Color,
    input_fg: Color,
    input_cursor: Color,
    selection_bg: Color,
    selection_fg: Color,
    title_fg: Color,
    font_width: u32,        // 8 (from VGA font)
    font_height: u32,       // 16 (from VGA font)
}
```

**Invariant:** Theme is immutable during a render pass. Widgets receive `&Theme` and use it for all color decisions. Theme can be swapped between frames.

---

## 7. Layer 4 — Window Manager

### 7.1 Window

```text
Window {
    id: u32,
    title: String,
    x: i32, y: i32,         // signed — windows can be partially off-screen
    width: u32, height: u32,
    buffer: OffscreenBuffer, // per-window pixel buffer
    dirty: bool,             // needs re-render into buffer
    visible: bool,
    focused: bool,
    decorations: bool,       // title bar + border if true
    // widget tree owned by the application, not the window
}
```

**Invariants:**
- Each window owns an `OffscreenBuffer` of exactly `width × height` pixels
- Windows render widgets into their own buffer — never directly to the framebuffer
- Only the compositor touches the framebuffer
- Window positions are `i32` to allow partial off-screen placement
- Window ID is monotonically increasing, never reused

### 7.2 Compositor

The compositor owns the framebuffer `Canvas` and a `Vec<Window>` in z-order (back to front).

**Composition strategy: damage-rect tracking.**

```text
DamageTracker {
    rects: Vec<Rect>,    // accumulated damage since last compose
    max_rects: usize,    // if exceeded, fall back to full-screen
}
```

**Composition loop (called per frame, ~60 FPS):**

1. Collect damage rects:
   - Window marked dirty → damage = window's screen rect
   - Window moved → damage = old rect ∪ new rect
   - Window resized → damage = old rect ∪ new rect
   - Window added/removed → damage = window rect
2. Merge overlapping damage rects (sweep-and-merge or simple union)
3. If damage rect count > threshold (8), union all into single full-screen rect
4. For each damage rect:
   - Fill with desktop background color
   - For each window (back to front) that intersects the damage rect:
     - Compute intersection of window buffer with damage rect
     - `blit` (opaque) or `blit_blend` (translucent) the intersection region

**Optimization — skip composition when nothing is damaged:**
If no windows are dirty and no windows moved, the compositor does zero work. This is the common case when the shell is idle.

**Optimization — opaque fast path:**
Most windows are fully opaque. The compositor checks `window.alpha == 255` and uses `blit` (memcpy) instead of `blit_blend` (per-pixel blend). This is the default.

**Optimization — bottom-up occlusion:**
Before compositing, walk z-order top-to-bottom. Mark fully-occluded regions. Skip blitting invisible windows entirely. For the common case of a single maximized window, this skips all background windows + desktop fill.

### 7.3 Focus & Input Routing

- Keyboard events → focused window's widget tree
- Mouse events → window under cursor (topmost in z-order at cursor position)
- Click on unfocused window → raise to top, set focus, then deliver click
- Tab within a window cycles focus among focusable widgets

### 7.4 Window Decorations

Title bar: 20px height (configurable via theme), drawn by compositor (not by window content).
Border: 1px, color from theme.
Close button: `[X]` in title bar right corner.
Drag handle: entire title bar.

Decorations are rendered by the compositor during composition, not stored in the window buffer. This keeps window buffers clean (content-only) and allows theme changes without re-rendering all window content.

---

## 8. Layer 5 — Shell & Desktop

### 8.1 Desktop

The desktop is the root surface. When no windows are open, it shows:
- Optional wallpaper (solid color in Phase 1)
- A **shell window** that auto-creates on boot

### 8.2 Shell

The shell is a special window containing:
- A `TextArea` widget for output history (ring buffer, scrollable)
- A `TextInput` widget for command input
- A command parser and dispatcher

**Built-in commands (Phase 1):**

| Command | Action |
|---------|--------|
| `help` | List available commands |
| `clear` | Clear shell output |
| `open <app>` | Spawn a windowed application |
| `close <id>` | Close window by ID |
| `list` | List open windows |
| `theme <name>` | Switch theme |
| `exit` | Return to firmware |

**Application registry:**

```text
AppEntry {
    name: &'static str,       // "distro-launcher"
    title: &'static str,      // "Distro Launcher"
    default_size: (u32, u32), // (640, 400)
    create: fn() -> Box<dyn App>,
}
```

The `App` trait:
```text
App trait:
    fn init(&mut self, canvas: &mut dyn Canvas, theme: &Theme)
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme)
    fn handle_event(&mut self, event: &Event) -> AppResult
    fn title(&self) -> &str
```

Each former menu item (Distro Launcher, Distro Downloader, Storage Manager, Installation) becomes an `App` implementation spawned via `open <name>`.

---

## 9. Event System

```text
Event enum:
    KeyPress { key: Key, modifiers: Modifiers }
    KeyRelease { key: Key, modifiers: Modifiers }
    MouseMove { x: i32, y: i32 }
    MousePress { button: MouseButton, x: i32, y: i32 }
    MouseRelease { button: MouseButton, x: i32, y: i32 }
    FocusGained
    FocusLost
    Resize { width: u32, height: u32 }
    Close
```

**Event flow:**
1. Hardware driver produces raw event (PS/2 scan code, mouse packet)
2. Event is translated to `Event` enum by the input layer (outside ui crate)
3. Event is delivered to `WindowManager::dispatch_event(event)`
4. WM routes to appropriate window based on focus/position
5. Window routes to focused widget
6. Widget returns `EventResult::Consumed` or `EventResult::Ignored`
7. Ignored events bubble up to window, then to WM for global shortcuts

---

## 10. Font System

### Phase 1: Embedded VGA 8×16

- Same `FONT_DATA: [[u8; 16]; 95]` from `morpheus-display`
- The UI crate embeds its own copy (or references it via the adapter)
- Glyph metrics: monospaced, 8×16, ASCII 0x20-0x7E
- No scaling, no Unicode, no anti-aliasing

### Phase 2 (future): PSF/BDF Loader

- Load .psf or .bdf fonts from byte slices
- Support 256-glyph and 512-glyph PSF2 tables
- Unicode mapping table support
- Multiple sizes (8×8, 8×14, 8×16, 16×32)

---

## 11. Performance Invariants & Optimization Strategies

### 11.1 Hard Invariants (never violated)

| # | Invariant |
|---|-----------|
| I1 | No floating point anywhere in the crate |
| I2 | No heap allocation during render passes (all allocs at init/resize) |
| I3 | No unbounded loops — every loop has a provable upper bound |
| I4 | `fill_rect` is always preferred over pixel-by-pixel for rectangular regions |
| I5 | Clipping is computed once per primitive, not per pixel |
| I6 | Damage tracking prevents redundant framebuffer writes |
| I7 | Opaque blit (memcpy) is used instead of blend whenever alpha == 255 |
| I8 | Stride and width are distinguished everywhere — never assumed equal |
| I9 | All coordinates clip silently — no panics on out-of-bounds |
| I10 | Color packing respects PixelFormat — never hardcode BGRX |

### 11.2 Optimization Strategies

| # | Strategy | Impact |
|---|----------|--------|
| O1 | **Row-level memset32 for fills** — `fill_rect` writes entire rows at once, not individual pixels | 100x fewer function calls for large fills |
| O2 | **Run-length glyph rendering** — collapse consecutive same-color pixels in each glyph row | 2× fewer calls for typical text |
| O3 | **Dirty-rect compositor** — only re-composite damaged screen regions | Near-zero CPU when idle |
| O4 | **Occlusion culling** — skip blitting windows fully covered by windows above them | Zero cost for hidden windows |
| O5 | **Per-window buffers** — compose from small per-window buffers, not a single full-screen double buffer | Memory: sum(window_sizes) << screen_size when windows are small |
| O6 | **Integer alpha blend** — `(src_c * src_a + dst_c * (255 - src_a) + 128) >> 8` with no division | Same quality as float, no FPU |
| O7 | **Opaque window fast path** — detect `alpha == 255` and use memcpy instead of per-pixel blend | Most windows skip blend entirely |
| O8 | **Solid glyph row shortcut** — rows 0xFF (full fg) and 0x00 (full bg) use single fill_rect(8px) | Top/bottom glyph padding is free |
| O9 | **Virtual scrolling in lists** — only render visible items, not entire list | O(visible) not O(total) |
| O10 | **Ring buffer for shell output** — fixed-size circular buffer, no reallocation on scroll | Constant memory for unlimited output |
| O11 | **Skip fully-clipped primitives** — bounding box check before any pixel math | Zero cost for off-screen draws |
| O12 | **Compositor batching** — merge adjacent damage rects to reduce per-rect overhead | Fewer blit setup calls |

### 11.3 Memory Budget

| Component | Memory |
|-----------|--------|
| 1920×1080 window buffer | 8,294,400 bytes (~8 MB) |
| 640×400 window buffer | 1,024,000 bytes (~1 MB) |
| 320×200 window buffer | 256,000 bytes (~256 KB) |
| Damage tracker (16 rects) | 128 bytes |
| Theme | ~64 bytes |
| Font data (95 glyphs × 16 bytes) | 1,520 bytes |
| Window metadata (per window) | ~128 bytes |
| Shell ring buffer (4096 lines × 120 chars) | ~491,520 bytes (~480 KB) |

Total for typical session (3 small windows + shell): ~4-5 MB heap

---

## 12. Module Structure

```text
ui/
├── Cargo.toml
└── src/
    ├── lib.rs              // crate root, re-exports
    ├── color.rs            // Color, PixelFormat, packing/unpacking
    ├── canvas.rs           // Canvas trait
    ├── buffer.rs           // OffscreenBuffer impl Canvas
    ├── clip.rs             // ClipCanvas wrapper
    ├── rect.rs             // Rect type, intersection, union
    ├── draw/
    │   ├── mod.rs          // re-exports
    │   ├── shapes.rs       // lines, rects, circles, rounded rects
    │   ├── glyph.rs        // glyph rendering (optimized)
    │   └── blit.rs         // blit and blend operations
    ├── font/
    │   ├── mod.rs
    │   └── vga8x16.rs      // embedded VGA font data
    ├── event.rs            // Event enum, Key, Modifiers, EventResult
    ├── theme.rs            // Theme struct + built-in themes
    ├── widget/
    │   ├── mod.rs          // Widget trait, re-exports
    │   ├── label.rs
    │   ├── button.rs
    │   ├── text_input.rs
    │   ├── text_area.rs
    │   ├── list.rs
    │   ├── panel.rs
    │   ├── progress.rs
    │   ├── divider.rs
    │   └── checkbox.rs
    ├── window.rs           // Window struct, decorations
    ├── compositor.rs       // DamageTracker, composition loop
    ├── wm.rs               // WindowManager: focus, z-order, input routing
    ├── shell/
    │   ├── mod.rs          // Shell struct, command parser
    │   ├── commands.rs     // built-in command implementations
    │   └── ring_buffer.rs  // fixed-size circular text buffer
    └── app.rs              // App trait, AppEntry registry
```

---

## 13. Trait Hierarchy

```text
Canvas                          (Layer 1 — pixel surface)
  ├── OffscreenBuffer           (heap-backed)
  ├── ClipCanvas<C: Canvas>     (sub-rect restrictor)
  └── [FramebufferCanvas]       (adapter in bootloader)

Widget                          (Layer 3 — UI element)
  ├── Label
  ├── Button
  ├── TextInput
  ├── TextArea
  ├── List
  ├── Panel
  ├── ProgressBar
  ├── Divider
  └── Checkbox

App                             (Layer 5 — windowed application)
  ├── ShellApp
  ├── DistroLauncherApp
  ├── DistroDownloaderApp
  ├── StorageManagerApp
  └── InstallerApp
```

---

## 14. Integration with MorpheusX Bootloader

### 14.1 Initialization Sequence

```text
1. UEFI GOP provides FramebufferInfo (base, size, width, height, stride, format)
2. EBS is called — UEFI services gone
3. Heap allocator initialized (linked_list_allocator)
4. FramebufferCanvas adapter created wrapping asm_fb_* primitives
5. WindowManager::new(framebuffer_canvas) initializes compositor
6. ShellWindow auto-spawned
7. Shell prompt appears, keyboard polling begins
8. User types `open distro-launcher` → WM spawns DistroLauncherApp in a new Window
```

### 14.2 Main Loop

```text
loop {
    if let Some(event) = input.poll() {
        wm.dispatch_event(event);
    }
    wm.compose();  // only does work if damage exists
    // yield / delay for frame limiting
}
```

### 14.3 Migration Path from Current TUI

The current `Screen` / `MainMenu` / `renderer.rs` code will be replaced entirely. The migration path:

1. Build `morpheus-ui` as a standalone crate
2. Implement `Canvas` for the framebuffer (adapter in bootloader)
3. Port each menu item to an `App` implementation
4. Replace `main_menu.run()` with `wm.run()`
5. Delete `renderer.rs`, `main_menu.rs`, `rain.rs`

---

## 15. What This Is NOT

- **Not a general-purpose desktop environment** — no multi-user, no IPC, no display server protocol
- **Not an X11/Wayland clone** — no client-server, no sockets, all in-process
- **Not GPU-accelerated** — pure CPU rendering to linear framebuffer
- **Not Unicode-ready in Phase 1** — ASCII 0x20-0x7E only, Unicode is Phase 2
- **Not a web renderer** — no CSS, no DOM, no layout engine

---

## 16. Phase Plan

### Phase 1 — Foundation (current target)
- Canvas trait + OffscreenBuffer + Color
- Drawing primitives (shapes, glyph, blit, blend)
- Font system (VGA 8×16)
- Theme system
- Widget toolkit (all Phase 1 widgets)
- Window + compositor + WM
- Shell + basic commands
- Framebuffer adapter in bootloader

### Phase 2 — Polish
- PS/2 mouse driver + cursor rendering
- Window drag/resize via mouse
- PSF/BDF font loader
- Unicode glyph support (Basic Latin + Latin-1 Supplement)
- VStack/HStack/Grid layouts
- Scrollbar widget
- Tab widget

### Phase 3 — Advanced
- Alpha-blended windows (translucency)
- Window animations (fade in/out via alpha ramp)
- Bitmap image display (BMP loader)
- Custom themes / theme files
- Multi-monitor support (multiple framebuffers)

---

## 17. Coding Conventions

- **Minimal comments** — code should be self-documenting via naming
- **Doc comments only on public items** and only when the name doesn't explain everything
- **No `unwrap()` in rendering paths** — use `if let` / `match` / saturating math
- **`#[inline]` on hot-path pixel operations** — `put_pixel`, `get_pixel`, `color_to_packed`
- **`const fn` wherever possible** — color constants, theme defaults
- **No `unsafe` except in the framebuffer adapter** — the UI crate itself is 100% safe Rust
- **All arithmetic uses `u32` for coordinates, `u8` for color channels** — no type confusion
- **`saturating_sub` / `min` / `max` instead of bounds checks + branches**
