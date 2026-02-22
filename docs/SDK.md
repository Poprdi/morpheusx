# MorpheusX SDK — Developer Reference

> **Platform**: Bare-metal x86-64 exokernel, no OS underneath  
> **Rust edition**: 2021 · `#![no_std]` · `extern crate alloc`  
> **Target**: `x86_64-unknown-uefi` (PE/COFF, MS x64 ABI)  
> **Last updated**: 2026-02-22

---

## Table of Contents

1. [Platform Overview](#1-platform-overview)
2. [App Framework](#2-app-framework)
3. [UI — Canvas & Color](#3-ui--canvas--color)
4. [UI — Draw Primitives](#4-ui--draw-primitives)
5. [UI — Widgets](#5-ui--widgets)
6. [UI — Theme](#6-ui--theme)
7. [UI — Events](#7-ui--events)
8. [UI — Shell](#8-ui--shell)
9. [UI — Window Manager](#9-ui--window-manager)
10. [UI — Font](#10-ui--font)
11. [Process & Scheduler](#11-process--scheduler)
12. [Signals](#12-signals)
13. [Syscall Interface](#13-syscall-interface)
14. [Memory Management](#14-memory-management)
15. [Heap Allocator](#15-heap-allocator)
16. [Paging & Virtual Memory](#16-paging--virtual-memory)
17. [Synchronization](#17-synchronization)
18. [Serial Debug Output](#18-serial-debug-output)
19. [DMA](#19-dma)
20. [PCI](#20-pci)
21. [CPU Primitives](#21-cpu-primitives)
22. [Disk — GPT Structures](#22-disk--gpt-structures)
23. [Disk — GPT Operations](#23-disk--gpt-operations)
24. [Disk — Partition Types](#24-disk--partition-types)
25. [Filesystem — FAT32 Format](#25-filesystem--fat32-format)
26. [Filesystem — FAT32 File Operations](#26-filesystem--fat32-file-operations)
27. [ISO Storage (Chunked)](#27-iso-storage-chunked)
28. [Network Stack](#28-network-stack)
29. [Crate Map & Import Paths](#29-crate-map--import-paths)
30. [Syscall Table (Quick Reference)](#30-syscall-table-quick-reference)

---

## 1. Platform Overview

MorpheusX is a bare-metal exokernel. After `ExitBootServices` the machine is
fully owned by the kernel. All apps run in Ring 0 alongside the kernel — there
is no system-call boundary cost for most operations. The architecture layers
from bottom to top:

```
 ┌─────────────────────────────────────────────────────┐
 │   App  (bootloader/src/apps/<name>.rs)              │
 │   implements App trait, renders to OffscreenBuffer  │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-ui      (ui/)    Window, Shell, Widgets  │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-core    (core/)  Disk, FS, ISO, Net-init │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-hwinit  (hwinit/)  Memory, Paging,       │
 │                              Process, Syscall, PCI  │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-display (display/) Framebuffer backend   │
 └─────────────────────────────────────────────────────┘
```

### Key invariants
- `#![no_std]` everywhere. Use `extern crate alloc` for `Vec`/`Box`/`String`.
- No floating-point. `panic = "abort"`.
- Single core. `spin::Mutex` used for future SMP support.
- Identity-mapped memory (physical address == virtual address) until paging is
  explicitly changed.
- All code runs in Ring 0. Syscall mechanism exists for future Ring 3 use.

---

## 2. App Framework

**Crate**: `morpheus-ui`  **Path**: `ui/src/app.rs`

Every app implements the `App` trait and registers itself in
`bootloader/src/apps/mod.rs`. The shell command `open <name>` launches it.

### `App` trait

```rust
pub trait App {
    /// Window title bar text.
    fn title(&self) -> &str;

    /// Preferred initial (width, height) in pixels.
    fn default_size(&self) -> (u32, u32);

    /// Called once when the window is first displayed.
    /// Use this to do initial layout / first render.
    fn init(&mut self, canvas: &mut dyn Canvas, theme: &Theme);

    /// Called every time the window needs to be redrawn (Tick or Redraw).
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme);

    /// Called for every event dispatched to this window.
    fn handle_event(&mut self, event: &Event) -> AppResult;
}
```

### `AppResult`

```rust
pub enum AppResult {
    Continue,   // Nothing changed; no redraw needed
    Close,      // App requests its window be closed
    Redraw,     // Canvas content changed; trigger re-composite
}
```

### `AppEntry` & registration

```rust
pub struct AppEntry {
    pub name:         &'static str,   // Shell command, e.g. "tasks"
    pub title:        &'static str,   // Human-readable title
    pub default_size: (u32, u32),
    pub create:       fn() -> Box<dyn App>,
}
```

Register in `bootloader/src/apps/mod.rs`:

```rust
pub mod my_app;
pub fn register_all(registry: &mut AppRegistry) {
    my_app::register(registry);
}

// In my_app.rs:
pub fn register(registry: &mut AppRegistry) {
    registry.register(AppEntry {
        name:         "myapp",
        title:        "My App",
        default_size: (640, 480),
        create:       || Box::new(MyApp::new()),
    });
}
```

### Minimal app skeleton

```rust
use alloc::boxed::Box;
use morpheus_ui::app::{App, AppEntry, AppRegistry, AppResult};
use morpheus_ui::canvas::Canvas;
use morpheus_ui::event::Event;
use morpheus_ui::theme::Theme;

pub struct MyApp;

impl MyApp {
    pub fn new() -> Self { MyApp }
}

impl App for MyApp {
    fn title(&self)        -> &str       { "My App" }
    fn default_size(&self) -> (u32, u32) { (400, 300) }

    fn init(&mut self, canvas: &mut dyn Canvas, theme: &Theme) {
        canvas.clear(theme.bg);
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        use morpheus_ui::draw::shapes::rect_fill;
        rect_fill(canvas, 10, 10, 100, 50, theme.accent);
    }

    fn handle_event(&mut self, event: &Event) -> AppResult {
        use morpheus_ui::event::{Event, Key};
        match event {
            Event::KeyPress(ke) if ke.key == Key::Escape => AppResult::Close,
            Event::Tick => AppResult::Redraw,
            _ => AppResult::Continue,
        }
    }
}

pub fn register(registry: &mut AppRegistry) {
    registry.register(AppEntry {
        name: "myapp", title: "My App",
        default_size: (400, 300),
        create: || Box::new(MyApp::new()),
    });
}
```

---

## 3. UI — Canvas & Color

**Crate**: `morpheus-ui`

### `Canvas` trait  (`ui/src/canvas.rs`)

```rust
pub trait Canvas {
    fn width(&self)  -> u32;
    fn height(&self) -> u32;
    fn stride(&self) -> u32;
    fn pixel_format(&self) -> PixelFormat;

    // Core pixel ops
    fn put_pixel(&mut self, x: u32, y: u32, color: Color);
    fn get_pixel(&self,     x: u32, y: u32) -> Color;
    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color);

    // Blit raw u32 pixel buffer (no blending)
    fn blit(&mut self, dst_x: u32, dst_y: u32,
            src: &[u32], src_w: u32, src_h: u32);

    // Blit with alpha blending
    fn blit_blend(&mut self, dst_x: u32, dst_y: u32,
                  src: &[u32], src_w: u32, src_h: u32, format: PixelFormat);

    // Convenience helpers (provided by default)
    fn bounds(&self) -> Rect;        // Rect::new(0,0,width,height)
    fn clear(&mut self, color: Color); // fill_rect full canvas
}
```

> **Note**: Always read `canvas.width()` / `canvas.height()` into local
> variables **before** passing `canvas` as `&mut` to avoid simultaneous borrow
> errors:
> ```rust
> let w = canvas.width();
> rect_fill(canvas, 0, 0, w, 10, theme.border); // ✓
> rect_fill(canvas, 0, 0, canvas.width(), 10, theme.border); // ✗ borrow error
> ```

### `OffscreenBuffer`  (`ui/src/buffer.rs`)

The concrete `Canvas` implementation used by every app window.

```rust
OffscreenBuffer::new(width: u32, height: u32, format: PixelFormat) -> Self
buf.pixels()     -> &[u32]      // raw packed pixel data
buf.pixels_mut() -> &mut [u32]
```

### `Color`  (`ui/src/color.rs`)

```rust
pub struct Color { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }

impl Color {
    // Constructors
    pub const fn rgb(r: u8, g: u8, b: u8)          -> Color;
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8)  -> Color;
    pub const fn with_alpha(self, a: u8)            -> Color;

    // Format conversion
    pub const fn to_packed(self, format: PixelFormat) -> u32;
    pub const fn from_packed(packed: u32, format: PixelFormat) -> Color;

    // Alpha blending: apply self over dst
    pub fn blend_over(self, dst: Color) -> Color;

    // Named constants
    pub const BLACK:       Color;
    pub const WHITE:       Color;
    pub const RED:         Color;
    pub const GREEN:       Color;   // 0x00AA00
    pub const BLUE:        Color;
    pub const YELLOW:      Color;   // 0xFFFF55
    pub const CYAN:        Color;
    pub const MAGENTA:     Color;
    pub const LIGHT_GRAY:  Color;
    pub const DARK_GRAY:   Color;
    pub const LIGHT_GREEN: Color;   // 0x55FF55
    pub const DARK_GREEN:  Color;   // 0x005500
    pub const TRANSPARENT: Color;   // rgba(0,0,0,0)
}
```

### `PixelFormat`

```rust
pub enum PixelFormat {
    Bgrx = 0,   // Most UEFI GOP framebuffers
    Rgbx = 1,
}
```

### `Rect`  (`ui/src/rect.rs`)

```rust
pub struct Rect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Rect;
    pub const fn zero() -> Rect;
    pub const fn is_empty(self) -> bool;
    pub const fn right(self)  -> u32;   // x + w
    pub const fn bottom(self) -> u32;   // y + h
    pub const fn contains(self, px: u32, py: u32) -> bool;
    pub const fn area(self) -> u32;
    pub fn intersect(self, other: Rect) -> Option<Rect>;
    pub fn union(self, other: Rect) -> Rect;
}
```

---

## 4. UI — Draw Primitives

**Crate**: `morpheus-ui`  **Import**: `morpheus_ui::draw::{shapes, glyph, blit}`

### Shapes  (`ui/src/draw/shapes.rs`)

All functions take `canvas: &mut dyn Canvas` as first parameter.

| Function | Signature | Description |
|----------|-----------|-------------|
| `hline` | `(canvas, x, y, w, color)` | Horizontal 1-px line |
| `vline` | `(canvas, x, y, h, color)` | Vertical 1-px line |
| `rect_fill` | `(canvas, x, y, w, h, color)` | Filled rectangle |
| `rect_outline` | `(canvas, x, y, w, h, thickness, color)` | Rectangle border only |
| `rounded_rect_fill` | `(canvas, x, y, w, h, radius, color)` | Filled rounded rect |
| `rounded_rect_outline` | `(canvas, x, y, w, h, radius, color)` | Rounded rect border |
| `circle_fill` | `(canvas, cx, cy, r, color)` | Filled circle |
| `circle_outline` | `(canvas, cx, cy, r, color)` | Circle border |
| `line` | `(canvas, x0, y0, x1, y1, color)` | Bresenham line |

### Glyph rendering  (`ui/src/draw/glyph.rs`)

```rust
// Draw a single character (8×16 bitmap font)
pub fn draw_char(
    canvas:    &mut dyn Canvas,
    x: u32, y: u32,
    c:    char,
    fg:   Color,
    bg:   Color,
    font: &[[u8; 16]],
);

// Draw a string (advances x by 8 per character; clips at canvas right edge)
pub fn draw_string(
    canvas:    &mut dyn Canvas,
    x: u32, y: u32,
    s:    &str,
    fg:   Color,
    bg:   Color,
    font: &[[u8; 16]],
);
```

> Pass `&font::FONT_DATA` for the built-in 8×16 VGA-style bitmap font.

---

## 5. UI — Widgets

**Crate**: `morpheus-ui`  **Import**: `morpheus_ui::widget::*`

All widgets implement the `Widget` trait:

```rust
pub trait Widget {
    fn size_hint(&self) -> (u32, u32);
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme);
    fn handle_event(&mut self, event: &Event) -> EventResult;
    fn is_focusable(&self) -> bool { false }
    fn set_focused(&mut self, focused: bool) {}
}
```

### `Label`

```rust
Label::new(text: &str) -> Label
label.set_text(&str)
label.text() -> &str
```

### `Button`

```rust
Button::new(label: &str) -> Button
button.set_label(&str)
button.was_pressed(&mut self) -> bool   // true once then resets
// Widget::handle_event returns EventResult::Consumed on click
```

### `TextInput`

```rust
TextInput::new(max_len: usize) -> TextInput
input.text()        -> &str
input.set_text(&str)
input.clear()
input.take_text(&mut self) -> String   // drains the buffer
// handles: printable chars, Backspace, Delete, Home, End, Left, Right
```

### `TextArea`

Multi-line read-only display area with scroll support.

```rust
TextArea::new(max_lines: usize) -> TextArea
area.push_line(&str)
area.clear()
// PageUp / PageDown scrolls
```

### `List`

Scrollable list of strings; Up/Down keys move selection.

```rust
List::new() -> List
List::with_items(items: Vec<String>) -> List
list.set_items(Vec<String>)
list.push(String)
list.clear()
list.selected_index() -> usize
list.selected_item()  -> Option<&str>
list.item_count()     -> usize
```

### `ProgressBar`

```rust
ProgressBar::new(width: u32) -> ProgressBar
bar.set_value(u32)
bar.set_max(u32)
bar.set_show_label(bool)   // shows "X%" text inside bar
bar.value()    -> u32
bar.fraction() -> u32      // 0-100
```

### `Panel`

Container that draws a title bar + border around child content.

```rust
Panel::new(title: &str, inner_w: u32, inner_h: u32) -> Panel
```

### `Divider`

Horizontal rule for visual separation.

```rust
Divider::new(width: u32) -> Divider
```

### `Checkbox`

```rust
Checkbox::new(label: &str, checked: bool) -> Checkbox
cb.is_checked() -> bool
cb.set_checked(bool)
// Space/Enter toggles; returns EventResult::Consumed
```

---

## 6. UI — Theme

**Crate**: `morpheus-ui`  **Import**: `morpheus_ui::theme::{Theme, THEME_DEFAULT}`

```rust
pub struct Theme {
    pub bg:               Color,   // Main background
    pub fg:               Color,   // Main foreground text
    pub accent:           Color,   // Highlighted elements
    pub border:           Color,   // Widget borders / separators
    pub button_bg:        Color,
    pub button_fg:        Color,
    pub button_focus_bg:  Color,
    pub input_bg:         Color,
    pub input_fg:         Color,
    pub input_cursor:     Color,
    pub selection_bg:     Color,
    pub selection_fg:     Color,
    pub title_fg:         Color,   // Window title text
    pub title_bg:         Color,   // Window title bar background
    pub scrollbar_bg:     Color,
    pub scrollbar_fg:     Color,
    pub font_width:       u32,     // Always 8
    pub font_height:      u32,     // Always 16
}

pub const THEME_DEFAULT: Theme = Theme {
    bg:              Color::BLACK,
    fg:              Color::LIGHT_GREEN,
    accent:          Color::GREEN,
    border:          Color::DARK_GREEN,
    button_bg:       Color::DARK_GREEN,
    button_fg:       Color::BLACK,
    button_focus_bg: Color::LIGHT_GREEN,
    input_bg:        Color { r:16, g:16, b:16, a:255 },
    input_fg:        Color::LIGHT_GREEN,
    input_cursor:    Color::LIGHT_GREEN,
    selection_bg:    Color::GREEN,
    selection_fg:    Color::BLACK,
    title_fg:        Color::BLACK,
    title_bg:        Color::DARK_GREEN,
    scrollbar_bg:    Color { r:24, g:24, b:24, a:255 },
    scrollbar_fg:    Color::DARK_GREEN,
    font_width:      8,
    font_height:     16,
};
```

---

## 7. UI — Events

**Crate**: `morpheus-ui`  **Import**: `morpheus_ui::event::*`

```rust
pub enum Event {
    KeyPress(KeyEvent),
    KeyRelease(KeyEvent),
    MouseMove   { x: i32, y: i32 },
    MousePress  { button: MouseButton, x: i32, y: i32 },
    MouseRelease{ button: MouseButton, x: i32, y: i32 },
    FocusGained,
    FocusLost,
    WindowResize{ width: u32, height: u32 },
    WindowClose,
    Tick,   // Fired on every PIT timer tick (~100 Hz); use for animation/polling
}

pub struct KeyEvent {
    pub key:       Key,
    pub modifiers: Modifiers,
}

pub enum Key {
    Char(char),
    Enter, Escape, Backspace, Delete, Tab,
    Up, Down, Left, Right,
    Home, End, PageUp, PageDown,
    F(u8),   // F1–F12
}

pub struct Modifiers {
    pub shift: bool,
    pub ctrl:  bool,
    pub alt:   bool,
}

pub enum MouseButton { Left, Right, Middle }

pub enum EventResult { Consumed, Ignored }
```

---

## 8. UI — Shell

**Crate**: `morpheus-ui`  **Path**: `ui/src/shell/`

The shell is embedded in the first window opened by the desktop event loop.

```rust
pub struct Shell;

impl Shell {
    pub fn new() -> Shell;
    pub fn push_output(&mut self, text: &str);
    pub fn render(&self, canvas: &mut dyn Canvas, theme: &Theme);
    pub fn handle_event(&mut self, event: &Event, window_ids: &[u32]) -> ShellAction;
}

pub enum ShellAction {
    None,
    OpenApp(String),       // name to look up in AppRegistry
    CloseWindow(u32),
    ListWindows,
    Exit,
}
```

### Built-in shell commands

| Command | Action |
|---------|--------|
| `help` | List commands |
| `open <name>` | Open app by registry name |
| `close <id>` | Close window by ID |
| `windows` | List open windows |
| `clear` | Clear output buffer |
| `exit` | Shut down |

---

## 9. UI — Window Manager

**Crate**: `morpheus-ui`  **Path**: `ui/src/wm.rs`

```rust
pub struct WindowManager;

impl WindowManager {
    pub fn new(
        screen_w: u32, screen_h: u32,
        format: PixelFormat,
        theme: &Theme,
    ) -> WindowManager;

    /// Create a new window, return its ID.
    pub fn create_window(
        &mut self,
        title: &str,
        x: i32, y: i32,
        width: u32, height: u32,
    ) -> u32;

    pub fn close_window(&mut self, id: u32);
    pub fn focus_window(&mut self, id: u32);

    /// Get the current window IDs (for shell command completion).
    pub fn window_ids(&self) -> Vec<u32>;

    /// Composite all windows to the framebuffer. Call every frame.
    pub fn compose(&mut self, fb: &mut dyn Canvas);

    /// Dispatch an event to the focused window.
    /// Returns None if no window handled it.
    pub fn dispatch_event(&mut self, event: &Event) -> Option<AppResult>;
}
```

> **OOM note**: Each window allocates an `OffscreenBuffer` which is
> `width × height × 4` bytes from the global heap. The heap grows
> dynamically from `MemoryRegistry` (up to 256 MB). At typical resolutions
> a 1022×746 shell + 800×500 app = ~4.6 MB — well within budget.

---

## 10. UI — Font

**Crate**: `morpheus-ui`  **Import**: `morpheus_ui::font`

```rust
// Built-in 8×16 VGA-style bitmap font (256 glyphs, CP437 encoding)
pub const FONT_DATA:   [[u8; 16]; 256];
pub const FONT_WIDTH:  u32 = 8;
pub const FONT_HEIGHT: u32 = 16;
```

Text width in pixels: `text.len() as u32 * FONT_WIDTH`  
Text height in pixels: `FONT_HEIGHT`

---

## 11. Process & Scheduler

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::{...}`

The scheduler runs preemptively at ~100 Hz (PIT IRQ 0). Context switching uses
the full `CpuContext` saved on the kernel stack by the timer ISR.

### Types

```rust
pub struct Process {
    pub pid:              u32,
    pub name:             [u8; 32],   // null-terminated UTF-8
    pub parent_pid:       Option<u32>,
    pub state:            ProcessState,
    pub exit_code:        Option<i32>,
    pub cr3:              u64,         // page table physical address
    pub kernel_stack_top: u64,
    pub kernel_stack_base:u64,
    pub context:          CpuContext,  // saved registers
    pub heap_region:      (u64, u64),  // (base, size)
    pub pages_allocated:  u64,
    pub priority:         u8,          // 0 = lowest, 255 = highest
    pub cpu_ticks:        u64,         // total ticks this process ran
    pub pending_signals:  SignalSet,
}

pub enum ProcessState {
    Ready,
    Running,
    Blocked(BlockReason),
    Zombie,
    Terminated,
}

pub enum BlockReason {
    Sleep(u64),         // wake after this tick count
    WaitChild(u32),     // waiting for child PID
    Io,
}

pub const MAX_PROCESSES: usize = 64;
```

### `CpuContext`  (`hwinit/src/process/context.rs`)

```rust
#[repr(C)]
pub struct CpuContext {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64,
    pub r8:  u64, pub r9:  u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub rflags: u64, pub rsp: u64,
    pub cs:  u64, pub ss:     u64,
}
// Total: 0xA0 (160) bytes
```

### `Scheduler` API  (`hwinit/src/process/scheduler.rs`)

```rust
pub static SCHEDULER: Scheduler;

impl Scheduler {
    /// Snapshot process table — allocation-free, safe from timer ISR.
    /// Returns number of entries written into `out`.
    pub fn snapshot_processes(&self, out: &mut [ProcessInfo]) -> usize;

    /// Number of currently alive processes.
    pub fn live_count(&self) -> u32;

    /// Currently running PID.
    pub fn current_pid(&self) -> u32;

    /// Total scheduler ticks since boot (~100 Hz).
    pub fn tick_count(&self) -> u32;

    /// Deliver a signal to a process.
    /// SIGKILL / SIGSTOP take effect immediately.
    /// Other signals are queued in pending_signals.
    pub unsafe fn send_signal(&self, pid: u32, sig: Signal) -> Result<(), &'static str>;
}

pub struct ProcessInfo {
    pub pid:         u32,
    pub name:        [u8; 32],
    pub state:       ProcessState,
    pub cpu_ticks:   u64,
    pub pages_alloc: u64,
    pub priority:    u8,
}
```

### Process lifecycle functions

```rust
/// Called once during platform init. Creates PID 0 (kernel thread).
pub unsafe fn init_scheduler();

/// Spawn a new kernel-mode thread.
/// `entry_fn`: function pointer that will be called as the thread entry.
/// Returns Err if the process table is full (MAX_PROCESSES = 64).
pub unsafe fn spawn_kernel_thread(
    name: &str,
    entry_fn: fn(),
    priority: u8,
) -> Result<u32, &'static str>;

/// Terminate the current process with exit code.
/// Marks as Zombie until parent calls waitpid.
pub unsafe fn exit_process(code: i32) -> !;

/// Timer ISR callback (called from ASM at 100 Hz).
/// Saves current context, picks next process, returns its context.
#[no_mangle]
pub unsafe extern "C" fn scheduler_tick(
    current_ctx: &CpuContext,
) -> &'static CpuContext;
```

---

## 12. Signals

**Import**: `morpheus_hwinit::{Signal, SignalSet}`

```rust
pub enum Signal {
    SIGINT  = 2,    // Ctrl+C; default action: terminate
    SIGKILL = 9,    // Unconditional kill; cannot be caught
    SIGSEGV = 11,   // Segmentation fault
    SIGTERM = 15,   // Graceful terminate request
    SIGCHLD = 17,   // Child state changed
    SIGCONT = 18,   // Resume a stopped process
    SIGSTOP = 19,   // Stop (pause) a process; cannot be caught
}

impl Signal {
    pub fn is_uncatchable(&self) -> bool;   // SIGKILL, SIGSTOP
    pub fn default_action(&self) -> &'static str;
    pub fn from_u8(n: u8) -> Option<Signal>;
}

/// Bitmask of pending signals (up to 64 distinct signals)
pub struct SignalSet(u64);

impl SignalSet {
    pub fn empty()                     -> SignalSet;
    pub fn raise(&mut self, sig: Signal);
    pub fn clear(&mut self, sig: Signal);
    pub fn is_pending(&self, sig: Signal) -> bool;
    pub fn take_next(&mut self) -> Option<Signal>;   // pops lowest pending
}
```

### Sending signals from an app

```rust
use morpheus_hwinit::{SCHEDULER, Signal};

unsafe {
    SCHEDULER.send_signal(pid, Signal::SIGTERM)?;
}
```

---

## 13. Syscall Interface

**Import**: `morpheus_hwinit::syscall::*`

The SYSCALL/SYSRET mechanism is configured on boot. Syscalls use the x86-64
ABI: number in `RAX`, arguments in `RDI, RSI, RDX, R10, R8, R9`.  
Return value in `RAX`. Errors returned as `u64::MAX - errno`.

Since all current apps run in Ring 0, direct Rust function calls are generally
preferred over the `syscall` instruction for performance. The syscall interface
is provided for completeness and future Ring 3 support.

### From Ring 0 (preferred — direct function call)

```rust
use morpheus_hwinit::process::scheduler::{exit_process, SCHEDULER};
exit_process(0);

SCHEDULER.send_signal(pid, Signal::SIGKILL);

// Allocate physical pages directly from registry
let registry = morpheus_hwinit::global_registry_mut();
let phys_addr = registry.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, n_pages)?;
```

### From Ring 3 (via `syscall` instruction)

```asm
; SYS_WRITE — write string to serial
mov rax, 1
mov rdi, buf_ptr
mov rsi, len
syscall
```

### Syscall dispatch — Rust side

```rust
pub unsafe extern "C" fn syscall_dispatch(
    nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64,
) -> u64;
```

---

## 14. Memory Management

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::memory::*`

The `MemoryRegistry` is the sole authority for physical memory post-EBS.

```rust
pub struct MemoryRegistry;

impl MemoryRegistry {
    // ── Query ──────────────────────────────────────────────────────
    pub fn total_memory(&self)     -> u64;   // bytes
    pub fn free_memory(&self)      -> u64;
    pub fn allocated_memory(&self) -> u64;
    pub fn bump_remaining(&self)   -> u64;   // bump allocator headroom
    pub fn region_count(&self)     -> usize;

    // ── Allocation ─────────────────────────────────────────────────
    /// Allocate `pages` 4KiB pages.
    /// Returns the physical base address on success.
    pub fn allocate_pages(
        &mut self,
        alloc_type: AllocateType,
        mem_type:   MemoryType,
        pages:      u64,
    ) -> Result<u64, MemoryError>;

    // ── Memory map export ──────────────────────────────────────────
    pub fn get_memory_map(&self) -> (&[MemoryDescriptor], usize);
    pub fn get_descriptor(&self, index: usize) -> Option<&MemoryDescriptor>;

    /// Export E820 table for Linux handoff.
    pub fn export_e820(&self, buf: &mut [E820Entry]) -> usize;
}
```

### Global registry access

```rust
/// True after init_global_registry() completes (Phase 1 hwinit).
pub fn is_registry_initialized() -> bool;

/// Immutable reference (panics if not initialized).
pub unsafe fn global_registry() -> &'static MemoryRegistry;

/// Mutable reference (panics if not initialized).
pub unsafe fn global_registry_mut() -> &'static mut MemoryRegistry;
```

### `AllocateType`

```rust
pub enum AllocateType {
    AnyPages,            // Allocate from any free region
    MaxAddress(u64),     // Allocate below a given physical address
    Address(u64),        // Allocate at a specific physical address
}
```

### `MemoryType`

```rust
pub enum MemoryType {
    Free,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    Allocated,           // Generic kernel allocation
    AllocatedStack,      // Kernel/IST stack pages
    AllocatedHeap,       // Heap pages
    AllocatedPageTable,  // PML4/PDPT/PD/PT pages
    AllocatedDma,        // DMA-coherent region
}
```

### Constants

```rust
pub const PAGE_SIZE:  usize = 4096;
pub const PAGE_SHIFT: usize = 12;
```

---

## 15. Heap Allocator

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::heap::*`

The hwinit `HeapAllocator` is Heap B (not the `#[global_allocator]`).  
The `#[global_allocator]` (`HybridAllocator` in `bootloader/`) automatically
grows from `MemoryRegistry` in 16 MB chunks up to 256 MB.

```rust
/// True after init_heap() / init_heap_with_buffer() completes.
pub fn is_heap_initialized() -> bool;

/// (total_bytes, used_bytes, free_bytes) — None if not initialized.
pub fn heap_stats() -> Option<(usize, usize, usize)>;

/// Initialize hwinit heap from MemoryRegistry (called in platform Phase 4).
pub unsafe fn init_heap(size_bytes: usize);

/// Initialize hwinit heap from a caller-supplied buffer (testing / custom).
pub unsafe fn init_heap_with_buffer(buf: *mut u8, size: usize);
```

---

## 16. Paging & Virtual Memory

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::{paging::*, ...}`

Post-Phase 8 the kernel page table singleton is live. Memory is initially
identity-mapped (phys == virt) from UEFI.

```rust
// ── Types ───────────────────────────────────────────────────────────────

pub struct PageFlags(u64);
impl PageFlags {
    pub const PRESENT:    PageFlags;
    pub const WRITABLE:   PageFlags;
    pub const USER:       PageFlags;
    pub const HUGE_PAGE:  PageFlags;   // 2 MiB page
    pub const GLOBAL:     PageFlags;
    pub const NO_EXECUTE: PageFlags;
    // Preset combos
    pub const KERNEL_RW:  PageFlags;   // PRESENT | WRITABLE | GLOBAL
    pub const KERNEL_RO:  PageFlags;   // PRESENT | GLOBAL
    pub const USER_CODE:  PageFlags;   // PRESENT | USER
    pub const USER_RW:    PageFlags;   // PRESENT | WRITABLE | USER
}

pub struct PageTableEntry(u64);
impl PageTableEntry {
    pub fn new(phys_frame: u64, flags: PageFlags) -> Self;
    pub fn phys_addr(&self) -> u64;
    pub fn is_present(&self) -> bool;
    pub fn is_huge(&self) -> bool;
}

pub struct VirtAddr {
    pub pml4_idx: usize,
    pub pdpt_idx: usize,
    pub pd_idx:   usize,
    pub pt_idx:   usize,
    pub offset:   usize,
}
impl VirtAddr {
    pub fn from_u64(virt: u64) -> Self;
    pub fn to_u64(&self) -> u64;
}

pub enum MappedPageSize { Page4K, Page2M }
```

### `PageTableManager`

```rust
impl PageTableManager {
    /// Read CR3 and wrap the current page tables.
    pub unsafe fn from_cr3() -> Self;

    pub fn cr3(&self) -> u64;

    /// Map a single 4 KiB page.
    pub unsafe fn map_4k(
        &mut self, virt: u64, phys: u64, flags: PageFlags,
    ) -> Result<(), &'static str>;

    /// Map a 2 MiB huge page.
    pub unsafe fn map_2m(
        &mut self, virt: u64, phys: u64, flags: PageFlags,
    ) -> Result<(), &'static str>;

    pub unsafe fn unmap_4k(&mut self, virt: u64) -> Result<(), &'static str>;
    pub unsafe fn unmap_2m(&mut self, virt: u64) -> Result<(), &'static str>;

    /// Walk the page table; returns mapped physical address or None.
    pub fn translate(&self, virt: u64) -> Option<u64>;

    /// Identity-map a contiguous range [base, base+size).
    pub unsafe fn identity_map_range(
        &mut self, base: u64, size: u64, flags: PageFlags,
    ) -> Result<(), &'static str>;
}
```

### Kernel page table convenience wrappers

```rust
pub fn is_paging_initialized()    -> bool;
pub unsafe fn init_kernel_page_table();

pub unsafe fn kmap_4k(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str>;
pub unsafe fn kmap_2m(virt: u64, phys: u64, flags: PageFlags) -> Result<(), &'static str>;
pub unsafe fn kunmap_4k(virt: u64) -> Result<(), &'static str>;
pub fn kvirt_to_phys(virt: u64)  -> Option<u64>;

pub unsafe fn kernel_page_table()     -> &'static PageTableManager;
pub unsafe fn kernel_page_table_mut() -> &'static mut PageTableManager;
```

---

## 17. Synchronization

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::{SpinLock, ...}`

```rust
/// RAII spinlock (disables interrupts on lock).
pub struct SpinLock<T>;
impl<T> SpinLock<T> {
    pub const fn new(val: T) -> Self;
    pub fn lock(&self) -> SpinLockGuard<T>;
    pub fn try_lock(&self) -> Option<SpinLockGuard<T>>;
}

/// Raw spinlock without data ownership.
pub struct RawSpinLock;
impl RawSpinLock {
    pub const fn new() -> Self;
    pub fn acquire(&self);
    pub fn release(&self);
}

/// One-time initialization cell.
pub struct Once;
impl Once {
    pub const fn new() -> Self;
    pub fn call_once<F: FnOnce()>(&self, f: F);
    pub fn is_completed(&self) -> bool;
}

/// Lazily-initialized value.
pub struct Lazy<T, F = fn() -> T>;
impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub const fn new(init: F) -> Self;
}
impl<T, F: FnOnce() -> T> Deref for Lazy<T, F> { type Target = T; }

/// RAII guard that saves/restores interrupt flag.
pub struct InterruptGuard;

/// Run closure with interrupts disabled; restores prior state after.
pub fn without_interrupts<F: FnOnce() -> R, R>(f: F) -> R;
```

---

## 18. Serial Debug Output

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::serial::*`

All output goes to COM1 (I/O port 0x3F8). Use freely in any no_std context —
these functions use only port I/O, no allocation.

```rust
pub fn putc(b: u8);             // Write single byte
pub fn puts(s: &str);           // Write string
pub fn put_hex8(val:  u8);      // "0xXX"
pub fn put_hex32(val: u32);     // "0xXXXXXXXX"
pub fn put_hex64(val: u64);     // "0xXXXXXXXXXXXXXXXX"
pub fn newline();               // Write '\n'
```

Example:

```rust
use morpheus_hwinit::serial::{puts, put_hex64};
puts("[MYAPP] buffer base = ");
put_hex64(buf_addr);
puts("\n");
```

---

## 19. DMA

**Crate**: `morpheus-hwinit`

A `DmaRegion` is a 32-bit-addressable contiguous buffer allocated below 4 GiB,
suitable for device DMA without address-translation.

```rust
pub struct DmaRegion {
    pub phys_base: u64,
    pub virt_base: u64,
    pub size:      usize,
}
impl DmaRegion {
    pub fn as_slice(&self) -> &[u8];
    pub fn as_slice_mut(&mut self) -> &mut [u8];
}
```

Allocated during platform Phase 6 (2 MB default). Access via the `PlatformInit`
struct returned by `platform_init_selfcontained()`.

---

## 20. PCI

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::pci::*`

All PCI config space accesses use Legacy PIO (CFC/CF8).

```rust
pub struct PciAddr {
    pub bus:  u8,
    pub dev:  u8,
    pub func: u8,
}
impl PciAddr {
    pub fn new(bus: u8, dev: u8, func: u8) -> Self;
}

// Config space reads
pub fn pci_cfg_read8 (addr: PciAddr, offset: u8) -> u8;
pub fn pci_cfg_read16(addr: PciAddr, offset: u8) -> u16;
pub fn pci_cfg_read32(addr: PciAddr, offset: u8) -> u32;

// Config space writes
pub fn pci_cfg_write8 (addr: PciAddr, offset: u8, val: u8);
pub fn pci_cfg_write16(addr: PciAddr, offset: u8, val: u16);
pub fn pci_cfg_write32(addr: PciAddr, offset: u8, val: u32);
```

Common config space offsets:

| Symbol | Value | Field |
|--------|-------|-------|
| `offset::VENDOR_ID`  | 0x00 | Vendor ID |
| `offset::DEVICE_ID`  | 0x02 | Device ID |
| `offset::COMMAND`    | 0x04 | Command register |
| `offset::CLASS_CODE` | 0x09 | Class / Subclass / Interface |
| `offset::BAR0`       | 0x10 | Base Address Register 0 |

---

## 21. CPU Primitives

**Crate**: `morpheus-hwinit`  **Import**: `morpheus_hwinit::cpu::{pio, mmio, barriers, tsc}`

### Port I/O  (`cpu::pio`)

```rust
pub unsafe fn outb(port: u16, val: u8);
pub unsafe fn outw(port: u16, val: u16);
pub unsafe fn outl(port: u16, val: u32);
pub unsafe fn inb(port: u16)  -> u8;
pub unsafe fn inw(port: u16)  -> u16;
pub unsafe fn inl(port: u16)  -> u32;
pub unsafe fn io_wait();   // 1-µs delay via dummy port write
```

### MMIO  (`cpu::mmio`)

```rust
pub unsafe fn mmio_read32(addr:  u64) -> u32;
pub unsafe fn mmio_write32(addr: u64, val: u32);
pub unsafe fn mmio_read16(addr:  u64) -> u16;
pub unsafe fn mmio_write16(addr: u64, val: u16);
pub unsafe fn mmio_read8(addr:   u64) -> u8;
pub unsafe fn mmio_write8(addr:  u64, val: u8);
```

### Memory barriers  (`cpu::barriers`)

```rust
pub fn sfence();   // Store fence (write ordering)
pub fn lfence();   // Load fence (read ordering)
pub fn mfence();   // Full memory fence
pub fn cache_clflush(addr: *const u8);
pub fn cache_flush_range(start: *const u8, len: usize);
```

### TSC  (`cpu::tsc`)

```rust
/// Calibrate TSC frequency using PIT. Returns Hz (e.g. ~3_000_000_000).
pub unsafe fn calibrate_tsc_pit() -> u64;

/// Read current TSC value.
#[inline(always)]
pub fn rdtsc() -> u64;

/// Delay for approximately `us` microseconds using TSC busy-loop.
pub fn delay_us(us: u64, tsc_freq_hz: u64);
```

### Interrupt control  (`cpu::idt`)

```rust
pub fn enable_interrupts();
pub fn disable_interrupts();
pub fn interrupts_enabled() -> bool;
```

---

## 22. Disk — GPT Structures

**Crate**: `morpheus-core`  **Path**: `core/src/disk/gpt.rs`

Low-level GPT on-disk types. For higher-level operations use `gpt_ops::*`.

```rust
#[repr(C, packed)]
pub struct GptHeader {
    pub signature:              [u8; 8],    // "EFI PART"
    pub revision:               u32,        // 0x00010000
    pub header_size:            u32,
    pub header_crc32:           u32,
    pub reserved:               u32,
    pub current_lba:            u64,
    pub backup_lba:             u64,
    pub first_usable_lba:       u64,
    pub last_usable_lba:        u64,
    pub disk_guid:              [u8; 16],
    pub partition_entry_lba:    u64,
    pub num_partition_entries:  u32,
    pub partition_entry_size:   u32,
    pub partition_array_crc32:  u32,
}

impl GptHeader {
    pub fn validate(&self) -> bool;
    pub fn from_bytes(data: &[u8]) -> Option<&GptHeader>;
}

#[repr(C, packed)]
pub struct GptPartitionEntry {
    pub partition_type_guid:   [u8; 16],
    pub unique_partition_guid: [u8; 16],
    pub starting_lba:          u64,
    pub ending_lba:            u64,
    pub attributes:            u64,
    pub partition_name:        [u16; 36],   // UTF-16LE
}

impl GptPartitionEntry {
    pub fn is_used(&self) -> bool;
    pub fn matches_type(&self, type_guid: &[u8; 16]) -> bool;
    pub fn get_name(&self) -> [u8; 36];   // simplified UTF-16→ASCII
}

pub struct GptPartitionTable<'a>;
impl<'a> GptPartitionTable<'a> {
    pub fn new(header: &'a GptHeader, entries_data: &'a [u8]) -> Self;
    pub fn get_entry(&self, index: u32) -> Option<&GptPartitionEntry>;
    pub fn find_by_type(&self, type_guid: &[u8; 16]) -> Option<&GptPartitionEntry>;
}
```

### Well-known type GUIDs

```rust
// EFI System Partition
pub const GUID_EFI_SYSTEM: [u8; 16];

// Linux data partition
pub const GUID_LINUX_FILESYSTEM: [u8; 16];

// Signature bytes
pub const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
```

---

## 23. Disk — GPT Operations

**Crate**: `morpheus-core`  **Path**: `core/src/disk/gpt_ops/`

All operations work over the `BlockIo` trait (implement it for your hardware).

```rust
pub enum GptError {
    IoError,
    InvalidHeader,
    NoSpace,
    PartitionNotFound,
    OverlappingPartitions,
    InvalidSize,
    AlignmentError,
}

pub struct FreeRegion {
    pub start_lba: u64,
    pub end_lba:   u64,
}
impl FreeRegion {
    pub fn size_lba(&self) -> u64;
    pub fn size_mb(&self)  -> u64;
}
```

### `scan_partitions`

```rust
use morpheus_core::disk::gpt_ops::scan::scan_partitions;
use morpheus_core::disk::partition::PartitionTable;

let mut table = PartitionTable::new();
scan_partitions(block_io, &mut table, 512)?;
// table now populated with PartitionInfo entries
for p in table.iter() {
    println!("{:?} start={} size={}MB", p.partition_type, p.start_lba, p.size_mb());
}
```

### `create_gpt`

Writes protective MBR + empty GPT with primary & backup headers.

```rust
use morpheus_core::disk::gpt_ops::create_modify::create_gpt;

create_gpt(block_io, num_blocks)?;
```

### `create_partition`

Adds a new partition into the next empty GPT slot.

```rust
use morpheus_core::disk::gpt_ops::create_modify::create_partition;
use morpheus_core::disk::partition::PartitionType;

create_partition(block_io, PartitionType::EfiSystem, start_lba, end_lba)?;
```

### `delete_partition`

```rust
use morpheus_core::disk::gpt_ops::create_modify::delete_partition;

delete_partition(block_io, partition_index)?;
```

### `find_free_regions`

```rust
use morpheus_core::disk::gpt_ops::find::find_free_regions;

let mut regions = [FreeRegion { start_lba: 0, end_lba: 0 }; 32];
let count = find_free_regions(block_io, &mut regions)?;
for r in &regions[..count] {
    println!("Free: {} LBAs = {} MB", r.size_lba(), r.size_mb());
}
```

### LBA helper

```rust
use morpheus_core::disk::gpt_ops::mb_to_lba;

let start = mb_to_lba(512);   // 512 MB → LBA
let end   = mb_to_lba(1024) - 1;
```

---

## 24. Disk — Partition Types

**Crate**: `morpheus-core`  **Path**: `core/src/disk/partition.rs`

```rust
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PartitionType {
    EfiSystem,
    LinuxFilesystem,
    LinuxSwap,
    BasicData,
    Unknown,
}

impl PartitionType {
    pub fn from_gpt_guid(guid: &GptPartitionType) -> Self;
    pub fn to_gpt_guid(&self) -> GptPartitionType;
}

pub struct PartitionInfo {
    pub index:          u32,
    pub partition_type: PartitionType,
    pub start_lba:      u64,
    pub end_lba:        u64,
}
impl PartitionInfo {
    pub fn size_mb(&self)    -> u64;
    pub fn type_name(&self)  -> &'static str;
}

/// Fixed-capacity partition table (max 16 partitions).
pub struct PartitionTable {
    pub has_gpt: bool,
}
impl PartitionTable {
    pub const fn new() -> Self;
    pub fn clear(&mut self);
    pub fn add_partition(&mut self, info: PartitionInfo) -> Result<(), ()>;
    pub fn count(&self) -> usize;
    pub fn get(&self, index: usize) -> Option<&PartitionInfo>;
    pub fn iter(&self) -> impl Iterator<Item = &PartitionInfo>;
}
```

---

## 25. Filesystem — FAT32 Format

**Crate**: `morpheus-core`  **Path**: `core/src/fs/fat32_format/`

Formats a raw partition (described by LBA start + sector count) as FAT32.

```rust
use morpheus_core::fs::fat32_format::format::format_fat32;

pub enum Fat32Error {
    IoError,
    PartitionTooSmall,   // < ~65 MB
    PartitionTooLarge,   // > 2 TB
    InvalidSector,
    AllocationFailed,
    InvalidCluster,
    PathTooLong,
    FileNotFound,
    DirectoryNotFound,
    FileTooLarge,
}

/// Format a partition as FAT32.
/// `block_io`: mutable reference to your BlockIo implementation.
/// `partition_lba_start`: first LBA of the partition.
/// `partition_sectors`:   total sector count of the partition.
pub fn format_fat32<B: BlockIo>(
    block_io:             &mut B,
    partition_lba_start:  u64,
    partition_sectors:    u64,
) -> Result<(), Fat32Error>;
```

Layout written:
- **LBA 0**: Boot sector (OEM name: `MORPHEUS`, label: `MORPHEUS   `)
- **LBA 1**: FSInfo sector  
- **LBA 6**: Backup boot sector  
- **LBA 32..32+FAT**: FAT1 (4-byte entries per cluster)  
- **LBA 32+FAT..32+FAT×2**: FAT2 (mirror)  
- **Cluster 2**: Root directory  
- **Cluster 3+**: Data area  

Cluster size: 8 sectors × 512 bytes = **4 KiB**.

### Verify

```rust
use morpheus_core::fs::fat32_format::verify::verify_fat32;

verify_fat32(block_io, partition_lba_start)?;
```

---

## 26. Filesystem — FAT32 File Operations

**Crate**: `morpheus-core`  **Path**: `core/src/fs/fat32_ops/`

All paths use `/`-separated components, rooted at the partition root.  
Maximum path depth: 8 components. Maximum pre-EBS file size: ~2 MiB (512 clusters).

```rust
use morpheus_core::fs::fat32_ops::{write_file, read_file, file_exists, create_directory};

// ── Write ─────────────────────────────────────────────────────────────────

/// Write `data` to `path`, creating directories as needed.
pub fn write_file<B: BlockIo>(
    block_io:            &mut B,
    partition_lba_start: u64,
    path:                &str,   // e.g. "/EFI/BOOT/BOOTX64.EFI"
    data:                &[u8],
) -> Result<(), Fat32Error>;

/// Same but reports progress via callback: `(bytes_written, total, message)`.
pub fn write_file_with_progress<B: BlockIo>(
    block_io:            &mut B,
    partition_lba_start: u64,
    path:                &str,
    data:                &[u8],
    progress:            &mut ProgressCallback,
) -> Result<(), Fat32Error>;

pub type ProgressCallback<'a> = Option<&'a mut dyn FnMut(usize, usize, &str)>;

// ── Read ──────────────────────────────────────────────────────────────────

/// Read the full contents of a file. Allocates a Vec<u8>.
pub fn read_file<B: BlockIo>(
    block_io:            &mut B,
    partition_lba_start: u64,
    path:                &str,
) -> Result<Vec<u8>, Fat32Error>;

// ── Existence & Directory ─────────────────────────────────────────────────

pub fn file_exists<B: BlockIo>(
    block_io:            &mut B,
    partition_lba_start: u64,
    path:                &str,
) -> Result<bool, Fat32Error>;

/// Create a directory (and all parent directories).
pub fn create_directory<B: BlockIo>(
    block_io:            &mut B,
    partition_lba_start: u64,
    path:                &str,
) -> Result<(), Fat32Error>;
```

### Complete example — format & write a file

```rust
use morpheus_core::fs::fat32_format::format::format_fat32;
use morpheus_core::fs::fat32_ops::write_file;

// 1. Format the partition
format_fat32(&mut blk, partition_start_lba, partition_sectors)?;

// 2. Write a file
let data = b"Hello, MorpheusX!";
write_file(&mut blk, partition_start_lba, "/hello.txt", data)?;

// 3. Read it back
let read_back = read_file(&mut blk, partition_start_lba, "/hello.txt")?;
assert_eq!(&read_back, data);
```

---

## 27. ISO Storage (Chunked)

**Crate**: `morpheus-core`  **Path**: `core/src/iso/`

Manages large ISO files that exceed the FAT32 4 GiB file size limit by
splitting them across multiple FAT32 chunk partitions.

```rust
pub const FAT32_MAX_FILE_SIZE: u64 = 0xFFFF_FFFF;        // 4 GiB - 1
pub const DEFAULT_CHUNK_SIZE:  u64 = 4 * 1024 * 1024 * 1024 - 4096;
pub const MAX_CHUNKS: usize = 16;

/// How many chunks for a given ISO size?
pub const fn chunks_needed(iso_size: u64, chunk_size: u64) -> usize;

/// Total disk space required (ISO + FAT32 overhead per chunk).
pub const fn disk_space_needed(iso_size: u64, chunk_size: u64) -> u64;
```

### Manifest

```rust
pub const MANIFEST_MAGIC: &str = "MORPHEUS_ISO_MANIFEST_V1";
pub const MAX_MANIFEST_SIZE: usize = 4096;

pub struct IsoManifest {
    pub name:      String,          // filename e.g. "ubuntu.iso"
    pub total_size: u64,
    pub chunk_count: usize,
    pub chunks:    [ChunkInfo; MAX_CHUNKS],
    pub sha256:    [u8; 32],        // optional hash
}

pub struct ChunkInfo {
    pub partition_uuid: [u8; 36],   // ASCII UUID string
    pub size:           u64,
}
```

Manifest is stored on the ESP at `/.iso/<name>.manifest`.

### Writing an ISO (during network download)

```rust
use morpheus_core::iso::writer::ChunkWriter;

let mut writer = ChunkWriter::new(block_io, &manifest)?;
// Feed data in arbitrary-size chunks:
while let Some(chunk) = next_http_chunk() {
    writer.write_chunk_data(&chunk)?;
}
writer.finalize()?;
```

### Reading an ISO (booting from it)

```rust
use morpheus_core::iso::reader::{ChunkReader, IsoReadContext};
use morpheus_core::iso::adapter::ChunkedBlockIo;

let reader  = ChunkReader::from_manifest(&manifest);
let blk     = ChunkedBlockIo::new(reader);
// blk now implements BlockIo presenting the whole ISO as a single device
let iso_img = IsoReadContext::new(blk);
let sector  = iso_img.read_sector(lba)?;
```

### Storage manager

```rust
use morpheus_core::iso::storage::{IsoStorageManager, MANIFEST_DIR};

pub const MANIFEST_DIR: &str = "/.iso";
pub const MAX_ISOS: usize = 8;

// List stored ISOs
let mgr = IsoStorageManager::new(block_io, esp_lba_start);
for entry in mgr.list()? {
    println!("{}: {} bytes", entry.name, entry.total_size);
}
```

---

## 28. Network Stack

**Crate**: `morpheus-network`

Network initialization runs **post-ExitBootServices** using a state machine
orchestrated in `network/src/mainloop/`.

### State machine flow

```
Init → GptPrep → LinkWait → DHCP → DNS → Connect → HTTP → Manifest → Done
                                                             ↓ (on error)
                                                           Failed
```

### States

| State | Purpose |
|-------|---------|
| `InitState` | Detect NIC, initialize VirtIO/Intel/Realtek driver |
| `GptPrepState` | Prepare GPT on target disk (create/scan partitions) |
| `LinkWaitState` | Wait for Ethernet link up (timeout: 10 s) |
| `DhcpState` | Obtain IP via DHCPv4 |
| `DnsState` | Resolve hostname → IPv4 |
| `ConnectState` | Open TCP connection to server |
| `HttpState` | HTTP/1.1 GET with chunked transfer |
| `ManifestState` | Parse manifest, set up ISO chunk partitions |
| `DoneState` | Transfer complete; manifest written to ESP |
| `FailedState` | Unrecoverable error; error log populated |

### Configuration

```rust
use morpheus_core::net::config::InitConfig;

pub struct InitConfig {
    pub ecam_base: Option<u64>,   // PCIe ECAM (None = use legacy PIO)
    pub server_ip:   [u8; 4],
    pub server_port: u16,
    pub url_path:    &'static str,
}

pub const ECAM_BASE_QEMU_I440FX: u64 = 0xB000_0000;
pub const ECAM_BASE_QEMU_Q35:    u64 = 0xB000_0000;
```

### `NetInterface`  (`network/src/stack/interface.rs`)

```rust
use morpheus_network::stack::{NetInterface, NetConfig};

impl NetInterface {
    pub fn new(device: impl NetworkDevice, config: NetConfig) -> Self;
    pub fn has_ip(&self) -> bool;
    pub fn ip_address(&self) -> Option<Ipv4Addr>;

    /// Drive the IP stack. Call at least once per ~10 ms.
    pub fn poll(&mut self, now_ms: u64);

    /// Open a TCP connection. Returns socket handle.
    pub fn tcp_connect(
        &mut self,
        remote: SocketAddrV4,
    ) -> Result<SocketHandle, NetworkError>;

    pub fn tcp_send(&mut self, handle: SocketHandle, data: &[u8]) -> Result<usize, NetworkError>;
    pub fn tcp_recv(&mut self, handle: SocketHandle, buf: &mut [u8]) -> Result<usize, NetworkError>;
    pub fn tcp_state(&self, handle: SocketHandle) -> TcpState;
    pub fn tcp_close(&mut self, handle: SocketHandle);

    /// DNS resolve (requires DNS socket in config).
    pub fn dns_start_query(&mut self, hostname: &str) -> Result<(), NetworkError>;
    pub fn dns_get_result(&mut self) -> Result<Option<Ipv4Addr>, GetQueryResultError>;
}
```

### Error log (ring buffer)

```rust
use morpheus_core::net::ring_buffer::*;

pub fn error_log(stage: InitStage, msg: &str);
pub fn debug_log(msg: &str);
pub fn error_log_pop() -> Option<ErrorLogEntry>;
pub fn error_log_count() -> usize;
pub fn error_log_clear();
pub fn drain_network_logs();   // drain logs from morpheus_network into this ring buffer

pub struct ErrorLogEntry {
    pub stage: InitStage,
    pub message: &'static str,
}

pub enum InitStage {
    General, PciScan, Driver, Link, Dhcp, Dns,
    TcpConnect, Http, IsoWrite, Manifest,
}
```

---

## 29. Crate Map & Import Paths

| What you need | Import |
|---------------|--------|
| App trait, AppResult | `morpheus_ui::app::{App, AppResult, AppEntry, AppRegistry}` |
| Canvas trait | `morpheus_ui::canvas::Canvas` |
| OffscreenBuffer | `morpheus_ui::buffer::OffscreenBuffer` |
| Color, PixelFormat | `morpheus_ui::color::{Color, PixelFormat}` |
| Rect | `morpheus_ui::rect::Rect` |
| Events | `morpheus_ui::event::{Event, Key, KeyEvent, Modifiers, MouseButton}` |
| Theme | `morpheus_ui::theme::{Theme, THEME_DEFAULT}` |
| Draw shapes | `morpheus_ui::draw::shapes::*` |
| Draw glyph | `morpheus_ui::draw::glyph::{draw_char, draw_string}` |
| Widgets | `morpheus_ui::widget::{Button, Label, TextInput, TextArea, List, Panel, ProgressBar, Divider, Checkbox}` |
| Font constants | `morpheus_ui::font::{FONT_DATA, FONT_WIDTH, FONT_HEIGHT}` |
| Shell | `morpheus_ui::shell::{Shell, ShellAction}` |
| Window Manager | `morpheus_ui::wm::WindowManager` |
| Process, Scheduler | `morpheus_hwinit::{Process, ProcessState, BlockReason, SCHEDULER, ProcessInfo, MAX_PROCESSES}` |
| spawn/exit | `morpheus_hwinit::{spawn_kernel_thread, exit_process}` |
| Signals | `morpheus_hwinit::{Signal, SignalSet}` |
| CpuContext | `morpheus_hwinit::CpuContext` |
| Syscall constants | `morpheus_hwinit::syscall::{SYS_EXIT, SYS_WRITE, ...}` |
| Memory registry | `morpheus_hwinit::{global_registry_mut, is_registry_initialized, MemoryType, AllocateType, PAGE_SIZE}` |
| Heap stats | `morpheus_hwinit::{is_heap_initialized, heap_stats}` |
| Paging | `morpheus_hwinit::{kmap_4k, kmap_2m, kunmap_4k, kvirt_to_phys, PageFlags, PageTableManager}` |
| Sync | `morpheus_hwinit::{SpinLock, Once, Lazy, without_interrupts, InterruptGuard}` |
| Serial debug | `morpheus_hwinit::serial::{puts, put_hex32, put_hex64, putc, newline}` |
| DMA | `morpheus_hwinit::dma::DmaRegion` |
| PCI | `morpheus_hwinit::pci::{PciAddr, pci_cfg_read8, pci_cfg_read16, pci_cfg_read32, pci_cfg_write8, pci_cfg_write16, pci_cfg_write32}` |
| Port I/O | `morpheus_hwinit::cpu::pio::{inb, outb, inw, outw, inl, outl, io_wait}` |
| MMIO | `morpheus_hwinit::cpu::mmio::{mmio_read32, mmio_write32}` |
| Barriers | `morpheus_hwinit::cpu::barriers::{sfence, lfence, mfence}` |
| TSC | `morpheus_hwinit::cpu::tsc::{rdtsc, delay_us, calibrate_tsc_pit}` |
| Interrupt control | `morpheus_hwinit::{enable_interrupts, disable_interrupts, interrupts_enabled}` |
| GPT structures | `morpheus_core::disk::gpt::{GptHeader, GptPartitionEntry, GptPartitionTable, GPT_SIGNATURE, GUID_EFI_SYSTEM, GUID_LINUX_FILESYSTEM}` |
| GPT scan | `morpheus_core::disk::gpt_ops::scan::scan_partitions` |
| GPT create/modify | `morpheus_core::disk::gpt_ops::create_modify::{create_gpt, create_partition, delete_partition}` |
| GPT find | `morpheus_core::disk::gpt_ops::find::{find_free_regions, FreeRegion}` |
| Partition types | `morpheus_core::disk::partition::{PartitionType, PartitionInfo, PartitionTable}` |
| FAT32 format | `morpheus_core::fs::fat32_format::format::format_fat32` |
| FAT32 verify | `morpheus_core::fs::fat32_format::verify::verify_fat32` |
| FAT32 file ops | `morpheus_core::fs::fat32_ops::{write_file, write_file_with_progress, read_file, file_exists, create_directory}` |
| ISO manifest | `morpheus_core::iso::{IsoManifest, ChunkInfo, MAX_CHUNKS}` |
| ISO writer | `morpheus_core::iso::writer::ChunkWriter` |
| ISO reader | `morpheus_core::iso::reader::{ChunkReader, IsoReadContext}` |
| ISO storage | `morpheus_core::iso::storage::{IsoStorageManager, MANIFEST_DIR}` |
| Network config | `morpheus_core::net::config::InitConfig` |
| Network errors | `morpheus_core::net::error_log::{error_log_pop, drain_network_logs}` |
| TCP/IP interface | `morpheus_network::stack::NetInterface` |

---

## 30. Syscall Table (Quick Reference)

| # | Name | Args | Return | Notes |
|---|------|------|--------|-------|
| 0 | `SYS_EXIT` | `(code: i32)` | `→ !` | Terminate current process |
| 1 | `SYS_WRITE` | `(ptr: *u8, len: usize, _)` | bytes written | Writes to COM1 serial |
| 2 | `SYS_READ` | `(fd, ptr, len)` | bytes read | Stub: returns 0 |
| 3 | `SYS_YIELD` | `()` | `0` | Voluntary context switch |
| 4 | `SYS_ALLOC` | `(pages: u64)` | phys base or `u64::MAX` | Allocate physical pages |
| 5 | `SYS_FREE` | `(phys: u64, pages: u64)` | `0` | Free physical pages (stub) |
| 6 | `SYS_GETPID` | `()` | current PID | |
| 7 | `SYS_KILL` | `(pid: u32, sig: u8)` | `0` or `u64::MAX` | Send signal to process |
| 8 | `SYS_WAIT` | `(pid: u32)` | exit code | Wait for child (stub) |
| 9 | `SYS_SLEEP` | `(ticks: u64)` | `0` | Block for N scheduler ticks |

**Error convention**: `u64::MAX` = generic error; `u64::MAX - 37` = ENOSYS (unknown syscall).

---

*End of MorpheusX SDK Reference*
