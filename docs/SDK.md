# MorpheusX SDK — Developer Reference

> **Platform**: Bare-metal x86-64 exokernel, no OS underneath  
> **Rust edition**: 2021 · `#![no_std]` · `extern crate alloc`  
> **Kernel target**: `x86_64-unknown-uefi` (PE/COFF, MS x64 ABI)  
> **User target**: `x86_64-morpheus.json` (ELF64, System V ABI, static)  
> **Last updated**: 2026-02-22

---

## Table of Contents

1. [Platform Overview](#1-platform-overview)
2. [App Framework (Ring 0)](#2-app-framework-ring-0)
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
31. [HelixFS — Log-Structured Filesystem](#31-helixfs--log-structured-filesystem)
32. [VFS — Virtual Filesystem Layer](#32-vfs--virtual-filesystem-layer)
33. [Ring 3 User Processes](#33-ring-3-user-processes)
34. [ELF Loader](#34-elf-loader)
35. [libmorpheus — Userspace SDK](#35-libmorpheus--userspace-sdk)
36. [Building Userspace Binaries](#36-building-userspace-binaries)
37. [stdin — Keyboard Input Buffer](#37-stdin--keyboard-input-buffer)
38. [Platform Capability Matrix](#38-platform-capability-matrix)

---

## 1. Platform Overview

MorpheusX is a bare-metal exokernel. After `ExitBootServices` the machine is
fully owned by the kernel. The system supports two execution models:

- **Ring 0 apps**: `App` trait implementations compiled into the kernel image.
  They run alongside the kernel with direct access to all hardware primitives.
  Used for system applications (storage manager, task manager).
- **Ring 3 user processes**: Standalone ELF64 binaries loaded from the HelixFS
  filesystem at runtime. Each process runs in its own address space with
  hardware-enforced isolation via x86-64 page tables. Communication with the
  kernel uses the `SYSCALL`/`SYSRET` mechanism (22 syscalls).

```
 ┌─────────────────────────────────────────────────────┐
 │   User Process  (Ring 3, own address space)         │
 │   linked against libmorpheus, uses SYSCALL ABI      │
 ├─────────────────────────────────────────────────────┤
 │   Ring 0 App  (bootloader/src/apps/<name>.rs)       │
 │   implements App trait, renders to OffscreenBuffer   │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-ui      (ui/)    Window, Shell, Widgets  │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-helix   (helix/) HelixFS + VFS layer     │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-core    (core/)  Disk, FAT32, ISO, Net   │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-hwinit  (hwinit/)  Memory, Paging,       │
 │        Process, Scheduler, Syscall, ELF, PCI, DMA   │
 ├─────────────────────────────────────────────────────┤
 │   morpheus-display (display/) Framebuffer backend   │
 └─────────────────────────────────────────────────────┘
```

### Key invariants
- `#![no_std]` everywhere. Use `extern crate alloc` for `Vec`/`Box`/`String`.
- No floating-point. `panic = "abort"`.
- Single core. `spin::Mutex` used for future SMP support.
- Kernel memory is identity-mapped (physical == virtual).
- User processes get isolated page tables (user-half only; kernel-half shared).
- Preemptive scheduler at ~100 Hz via PIT timer; round-robin.
- 22 syscalls (process, I/O, filesystem, signals) via `SYSCALL`/`SYSRET`.
- Per-process file descriptor tables (64 fds each) backed by HelixFS.
- Keyboard input flows to both focused Ring 0 apps and a shared stdin ring
  buffer for Ring 3 processes.

### Boot sequence (abridged)

```
efi_main() → ExitBootServices → switch_to_post_ebs()
  → platform_init_selfcontained()
      Phase 1:  MemoryRegistry (1958 MB bump allocator)
      Phase 2:  GDT + IDT (kernel, user, TSS segments)
      Phase 3:  PIC (8259)
      Phase 4:  Heap (4 MB from registry)
      Phase 5:  TSC calibration (~3 GHz)
      Phase 6:  DMA region (2 MB below 4 GiB)
      Phase 7:  PCI bus mastering
      Phase 8:  Kernel page table (identity-mapped)
      Phase 9:  Scheduler (PID 0 = kernel)
      Phase 10: SYSCALL/SYSRET MSRs
      Phase 11: HelixFS root filesystem (16 MB RAM disk)
  → run_desktop()
```

---

## 2. App Framework (Ring 0)

**Crate**: `morpheus-ui`  **Path**: `ui/src/app.rs`

Ring 0 apps run inside the kernel address space with full hardware access.
Every app implements the `App` trait and registers itself in
`bootloader/src/apps/mod.rs`. The shell command `open <name>` launches it.

For Ring 3 user processes, see §33–§36.

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
    SpawnProcess(String),  // load ELF from /bin/<name> and spawn Ring 3 process
    Exit,
}
```

### Built-in shell commands

| Command | Action |
|---------|--------|
| `help` | List commands |
| `open <name>` | Open Ring 0 app by registry name |
| `exec <name>` | Spawn Ring 3 user process from `/bin/<name>` |
| `run <name>` | Alias for `exec` |
| `close <id>` | Close window by ID |
| `list` / `windows` | List open windows |
| `clear` | Clear output buffer |
| `exit` | Shut down |

### `exec` / `run` — spawning user processes

The `exec` command loads an ELF64 binary from the HelixFS filesystem at
`/bin/<name>`, parses its PT_LOAD segments, creates a fresh page table
with the kernel mappings cloned, maps the segments + user stack, and adds
the process to the scheduler as Ready. The process runs in Ring 3 with
its own address space.

```
morpheus> exec hello
Spawned 'hello' as PID 3
```

If the binary is not found, the shell reports:
```
Binary not found: hello. Place ELF in /bin/hello
```

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
the full `CpuContext` saved on the kernel stack by the timer ISR. On each tick
the ISR also performs a CR3 switch if the next process has a different address
space.

### Execution model

- **PID 0**: The kernel itself (desktop event loop). Always present, never terminates.
- **Kernel threads** (Ring 0): Spawned via `spawn_kernel_thread()`. Share the
  kernel address space (same CR3). Have private kernel stacks.
- **User processes** (Ring 3): Spawned via `spawn_user_process()`. Each has
  its own page table (kernel-half cloned, user-half private). Transitions to
  Ring 3 via `SYSRET` from the first context switch.

### Types

```rust
pub struct Process {
    pub pid:              u32,
    pub name:             [u8; 32],   // null-terminated UTF-8
    pub parent_pid:       u32,
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
    pub fd_table:         FdTable,     // 64 per-process file descriptors
}

pub enum ProcessState {
    Ready,
    Running,
    Blocked(BlockReason),
    Zombie,
    Terminated,
}

pub enum BlockReason {
    Sleep(u64),         // wake after this TSC deadline
    WaitChild(u32),     // waiting for child PID to exit
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

    /// Per-process file descriptor table (for VFS operations).
    pub unsafe fn current_fd_table_mut(&self) -> &'static mut FdTable;

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

/// Spawn a new kernel-mode thread (Ring 0, shared address space).
pub unsafe fn spawn_kernel_thread(
    name: &str,
    entry_fn: u64,
    priority: u8,
) -> Result<u32, &'static str>;

/// Spawn a Ring 3 user process from an ELF64 binary.
/// Creates a fresh page table, loads PT_LOAD segments, maps user stack.
pub unsafe fn spawn_user_process(
    name: &str,
    elf_data: &[u8],
) -> Result<u32, &'static str>;

/// Terminate the current process with exit code.
/// Marks as Zombie until parent calls waitpid.
pub unsafe fn exit_process(code: i32) -> !;

/// Block the current process until a TSC deadline.
/// Used by SYS_SLEEP. The timer ISR wakes the process once the
/// deadline passes.
pub unsafe fn block_sleep(deadline: u64) -> u64;

/// Wait for a child process to exit and reap it.
/// If already Zombie, reaps immediately. Otherwise blocks with
/// BlockReason::WaitChild(pid).
pub unsafe fn wait_for_child(child_pid: u32) -> u64;

/// Store TSC frequency for sleep deadline computation.
/// Called once from platform init Phase 5.
pub unsafe fn set_tsc_frequency(freq: u64);

/// Get the stored TSC frequency (Hz). Returns 0 if not calibrated.
pub fn tsc_frequency() -> u64;

/// Timer ISR callback (called from ASM at 100 Hz).
/// Saves current context, wakes expired sleepers, picks next process,
/// writes `next_cr3` for address space switch, returns next context.
#[no_mangle]
pub unsafe extern "C" fn scheduler_tick(
    current_ctx: &CpuContext,
) -> &'static CpuContext;
```

### Context switch flow (timer ISR)

```
PIT IRQ 0 fires → irq_timer_isr (ASM)
  ├── Push all GPRs onto stack → CpuContext
  ├── Call scheduler_tick(current_ctx)
  │     ├── Save context into PROCESS_TABLE[current_pid]
  │     ├── Wake expired sleepers (TSC deadline comparison)
  │     ├── Round-robin pick_next()
  │     ├── Set kernel stack pointer for Ring 3→0 transitions
  │     ├── Write next_cr3 (process page table address)
  │     └── Return &next_process.context
  ├── Load next_cr3 into CR3 (if changed)
  ├── Restore all GPRs from returned context
  └── iretq → resumes next process
```

### Process cleanup on exit

When a process exits (via `SYS_EXIT` or `SIGKILL`):
1. State → `Zombie`, exit code recorded
2. Parent is woken if blocked on `WaitChild`
3. `SIGCHLD` delivered to parent
4. On reap (`SYS_WAIT` from parent):
   - Kernel stack pages freed to MemoryRegistry
   - User page tables walked (PML4→PDPT→PD→PT, indices 0–255 only)
   - All user-half physical frames freed
   - All intermediate page table pages freed
   - PML4 page itself freed
   - State → `Terminated`, slot available for reuse

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

### GDT segment layout (required for SYSRET)

| Selector | Segment |
|----------|---------|
| 0x00 | Null |
| 0x08 | Kernel Code (Ring 0, CS) |
| 0x10 | Kernel Data (Ring 0, SS) |
| 0x18 | User Data (Ring 3, SS = 0x1B) |
| 0x20 | User Code (Ring 3, CS = 0x23) |
| 0x28 | TSS |

> `SYSRET` requires User Data at `STAR[63:48]` and User Code at `STAR[63:48]+16`.
> The GDT order above satisfies this constraint.

### From Ring 0 (direct function call — preferred for kernel apps)

```rust
use morpheus_hwinit::process::scheduler::{exit_process, SCHEDULER};
exit_process(0);

SCHEDULER.send_signal(pid, Signal::SIGKILL);

// Allocate physical pages directly
let registry = morpheus_hwinit::global_registry_mut();
let phys = registry.allocate_pages(AllocateType::AnyPages, MemoryType::Allocated, n)?;
```

### From Ring 3 (via `syscall` instruction)

```asm
; SYS_WRITE — write string to serial (fd 1 = stdout)
mov rax, 1        ; SYS_WRITE
mov rdi, 1        ; fd = stdout
mov rsi, buf_ptr  ; pointer to data
mov rdx, len      ; byte count
syscall           ; RAX = bytes written
```

### From Ring 3 (via libmorpheus)

```rust
use libmorpheus::io::println;
use libmorpheus::fs;
use libmorpheus::process;

println("Hello from Ring 3!");
let fd = fs::open("/hello.txt", fs::O_READ)?;
let n = fs::read(fd, &mut buf)?;
fs::close(fd)?;
process::exit(0);
```

### Syscall dispatch — Rust side

```rust
#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64,
) -> u64;
```

### Error convention

| Value | Meaning |
|-------|---------|
| `0`..`0xFFFF_FFFF_FFFF_FF00` | Success / data |
| `u64::MAX` (0xFFFF...FFFF) | `-EINVAL` |
| `u64::MAX - 2` | `-ENOENT` |
| `u64::MAX - 3` | `-ESRCH` |
| `u64::MAX - 5` | `-EIO` |
| `u64::MAX - 9` | `-EBADF` |
| `u64::MAX - 10` | `-ECHILD` |
| `u64::MAX - 12` | `-ENOMEM` |
| `u64::MAX - 13` | `-EACCES` |
| `u64::MAX - 17` | `-EEXIST` |
| `u64::MAX - 21` | `-EISDIR` |
| `u64::MAX - 22` | `-EINVAL` (signal) |
| `u64::MAX - 24` | `-EMFILE` |
| `u64::MAX - 28` | `-ENOSPC` |
| `u64::MAX - 30` | `-EROFS` |
| `u64::MAX - 37` | `-ENOSYS` |
| `u64::MAX - 39` | `-ENOTEMPTY` |

Check with: `if ret > 0xFFFF_FFFF_FFFF_FF00 { /* error */ }`

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
| **UI — App Framework** | |
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
| **Process & Signals** | |
| Process, Scheduler | `morpheus_hwinit::{Process, ProcessState, BlockReason, SCHEDULER, ProcessInfo, MAX_PROCESSES}` |
| spawn kernel thread | `morpheus_hwinit::process::scheduler::spawn_kernel_thread` |
| spawn user process | `morpheus_hwinit::process::scheduler::spawn_user_process` |
| exit | `morpheus_hwinit::process::scheduler::exit_process` |
| sleep blocking | `morpheus_hwinit::process::scheduler::block_sleep` |
| wait for child | `morpheus_hwinit::process::scheduler::wait_for_child` |
| TSC freq get/set | `morpheus_hwinit::process::scheduler::{tsc_frequency, set_tsc_frequency}` |
| Signals | `morpheus_hwinit::{Signal, SignalSet}` |
| CpuContext | `morpheus_hwinit::CpuContext` |
| **Syscalls** | |
| Syscall numbers | `morpheus_hwinit::syscall::{SYS_EXIT, SYS_WRITE, ..., SYS_VERSIONS}` |
| Init | `morpheus_hwinit::syscall::init_syscall` |
| **Memory & Paging** | |
| Memory registry | `morpheus_hwinit::{global_registry_mut, is_registry_initialized, MemoryType, AllocateType, PAGE_SIZE}` |
| Heap stats | `morpheus_hwinit::{is_heap_initialized, heap_stats}` |
| Paging | `morpheus_hwinit::{kmap_4k, kmap_2m, kunmap_4k, kvirt_to_phys, PageFlags, PageTableManager}` |
| ELF loader | `morpheus_hwinit::elf::{load_elf64, validate_elf64, ElfImage, ElfError}` |
| **Sync & CPU** | |
| Sync | `morpheus_hwinit::{SpinLock, Once, Lazy, without_interrupts, InterruptGuard}` |
| Serial debug | `morpheus_hwinit::serial::{puts, put_hex32, put_hex64, putc, newline}` |
| stdin buffer | `morpheus_hwinit::stdin::{push, read, available}` |
| DMA | `morpheus_hwinit::dma::DmaRegion` |
| PCI | `morpheus_hwinit::pci::{PciAddr, pci_cfg_read8, pci_cfg_read16, pci_cfg_read32, pci_cfg_write8, pci_cfg_write16, pci_cfg_write32}` |
| Port I/O | `morpheus_hwinit::cpu::pio::{inb, outb, inw, outw, inl, outl, io_wait}` |
| MMIO | `morpheus_hwinit::cpu::mmio::{mmio_read32, mmio_write32}` |
| Barriers | `morpheus_hwinit::cpu::barriers::{sfence, lfence, mfence}` |
| TSC | `morpheus_hwinit::cpu::tsc::{rdtsc, delay_us, calibrate_tsc_pit, read_tsc}` |
| Interrupt control | `morpheus_hwinit::{enable_interrupts, disable_interrupts, interrupts_enabled}` |
| **Disk & FAT32** | |
| GPT structures | `morpheus_core::disk::gpt::{GptHeader, GptPartitionEntry, GptPartitionTable, GPT_SIGNATURE, GUID_EFI_SYSTEM, GUID_LINUX_FILESYSTEM}` |
| GPT scan | `morpheus_core::disk::gpt_ops::scan::scan_partitions` |
| GPT create/modify | `morpheus_core::disk::gpt_ops::create_modify::{create_gpt, create_partition, delete_partition}` |
| GPT find | `morpheus_core::disk::gpt_ops::find::{find_free_regions, FreeRegion}` |
| Partition types | `morpheus_core::disk::partition::{PartitionType, PartitionInfo, PartitionTable}` |
| FAT32 format | `morpheus_core::fs::fat32_format::format::format_fat32` |
| FAT32 verify | `morpheus_core::fs::fat32_format::verify::verify_fat32` |
| FAT32 file ops | `morpheus_core::fs::fat32_ops::{write_file, write_file_with_progress, read_file, file_exists, create_directory}` |
| **ISO & Network** | |
| ISO manifest | `morpheus_core::iso::{IsoManifest, ChunkInfo, MAX_CHUNKS}` |
| ISO writer | `morpheus_core::iso::writer::ChunkWriter` |
| ISO reader | `morpheus_core::iso::reader::{ChunkReader, IsoReadContext}` |
| ISO storage | `morpheus_core::iso::storage::{IsoStorageManager, MANIFEST_DIR}` |
| Network config | `morpheus_core::net::config::InitConfig` |
| Network errors | `morpheus_core::net::error_log::{error_log_pop, drain_network_logs}` |
| TCP/IP interface | `morpheus_network::stack::NetInterface` |
| **Filesystem (HelixFS + VFS)** | |
| VFS operations | `morpheus_helix::vfs::{vfs_open, vfs_read, vfs_write, vfs_close, vfs_seek, vfs_stat, vfs_readdir, vfs_mkdir, vfs_unlink, vfs_rename, vfs_sync}` |
| VFS types | `morpheus_helix::vfs::{FdTable, MountTable, MountEntry, FsInstance}` |
| Global FS | `morpheus_helix::vfs::global::{init_root_fs, fs_global, fs_global_mut}` |
| On-disk types | `morpheus_helix::types::{HelixSuperblock, FileStat, DirEntry, IndexEntry, LogOp}` |
| Open flags | `morpheus_helix::types::open_flags::{O_READ, O_WRITE, O_CREATE, O_TRUNC, O_APPEND, O_DIR, O_AT_LSN}` |
| Errors | `morpheus_helix::error::HelixError` |
| Block device | `morpheus_helix::device::MemBlockDevice` |
| Ops layer | `morpheus_helix::ops::{read, write, dir}` |
| **Userspace SDK (libmorpheus)** | |
| Entry macro | `libmorpheus::entry` |
| Raw syscalls | `libmorpheus::raw::{syscall0..syscall5, SYS_*}` |
| File ops | `libmorpheus::fs::{open, read, write, close, seek, mkdir, unlink, rename, stat, sync}` |
| Process ops | `libmorpheus::process::{exit, getpid, yield_cpu, kill, sleep}` |
| Console I/O | `libmorpheus::io::{print, println}` |
| Error check | `libmorpheus::is_error` |

---

## 30. Syscall Table (Quick Reference)

### Core syscalls (0–9)

| # | Name | Args | Return | Status |
|---|------|------|--------|--------|
| 0 | `SYS_EXIT` | `(code: i32)` | `→ !` | ✅ Implemented |
| 1 | `SYS_WRITE` | `(fd, ptr: *u8, len)` | bytes written | ✅ fd 1/2 → serial, fd ≥ 3 → VFS |
| 2 | `SYS_READ` | `(fd, ptr: *u8, len)` | bytes read | ✅ fd 0 → stdin, fd ≥ 3 → VFS |
| 3 | `SYS_YIELD` | `()` | `0` | ✅ STI+HLT+CLI (atomic yield) |
| 4 | `SYS_ALLOC` | `(pages: u64)` | phys base | ✅ Max 1024 pages per call |
| 5 | `SYS_FREE` | `(phys: u64, pages)` | `0` | ⚠️ Stub (returns 0, no-op) |
| 6 | `SYS_GETPID` | `()` | current PID | ✅ Implemented |
| 7 | `SYS_KILL` | `(pid: u32, sig: u8)` | `0` or error | ✅ All signals supported |
| 8 | `SYS_WAIT` | `(pid: u32)` | exit code | ✅ Blocks or reaps zombie |
| 9 | `SYS_SLEEP` | `(millis: u64)` | `0` | ✅ TSC-deadline based |

### HelixFS syscalls (10–21)

| # | Name | Args | Return | Status |
|---|------|------|--------|--------|
| 10 | `SYS_OPEN` | `(path_ptr, path_len, flags)` | fd | ✅ O_READ, O_WRITE, O_CREATE, O_TRUNC, O_APPEND |
| 11 | `SYS_CLOSE` | `(fd)` | `0` | ✅ Implemented |
| 12 | `SYS_SEEK` | `(fd, offset: i64, whence)` | new offset | ✅ SEEK_SET/CUR/END |
| 13 | `SYS_STAT` | `(path_ptr, path_len, buf_ptr)` | `0` | ✅ Writes FileStat to buf |
| 14 | `SYS_READDIR` | `(path_ptr, path_len, buf_ptr)` | count | ✅ Writes DirEntry[] to buf |
| 15 | `SYS_MKDIR` | `(path_ptr, path_len)` | `0` | ✅ Implemented |
| 16 | `SYS_UNLINK` | `(path_ptr, path_len)` | `0` | ✅ Files and empty dirs |
| 17 | `SYS_RENAME` | `(old_ptr, old_len, new_ptr, new_len)` | `0` | ✅ Implemented |
| 18 | `SYS_TRUNCATE` | `(fd, new_size)` | `0` | ❌ Stub (ENOSYS) |
| 19 | `SYS_SYNC` | `()` | `0` | ✅ Flushes log + superblock |
| 20 | `SYS_SNAPSHOT` | `(name_ptr, name_len)` | snapshot_id | ❌ Stub (ENOSYS) |
| 21 | `SYS_VERSIONS` | `(path_ptr, path_len, buf, max)` | count | ❌ Stub (ENOSYS) |

### Open flags

| Flag | Value | Description |
|------|-------|-------------|
| `O_READ` | 0x01 | Open for reading |
| `O_WRITE` | 0x02 | Open for writing |
| `O_CREATE` | 0x04 | Create if not exists |
| `O_TRUNC` | 0x10 | Truncate to zero on open |
| `O_APPEND` | 0x20 | Append mode |
| `O_DIR` | 0x40 | Open as directory |
| `O_AT_LSN` | 0x80 | Time-travel read at specific LSN |

### Seek whence

| Constant | Value | Description |
|----------|-------|-------------|
| `SEEK_SET` | 0 | From beginning |
| `SEEK_CUR` | 1 | From current offset |
| `SEEK_END` | 2 | From end of file |

---

## 31. HelixFS — Log-Structured Filesystem

**Crate**: `morpheus-helix`  **Path**: `helix/`

HelixFS is a log-structured, copy-on-write filesystem designed for MorpheusX.
All writes are appended to a circular log; the on-disk state is always
consistent and never requires fsck. It supports time-travel reads at
historical log sequence numbers (LSNs).

### Architecture

```
 ┌──────────────────────────────────────────────────┐
 │  Superblock (4 KiB, block 0)                     │
 ├──────────────────────────────────────────────────┤
 │  Block bitmap (tracks free/used blocks)           │
 ├──────────────────────────────────────────────────┤
 │  Log area (circular, append-only)                 │
 │    ├── LogSegmentHeader (64B) per segment         │
 │    └── LogRecordHeader (64B) + payload per op     │
 ├──────────────────────────────────────────────────┤
 │  Index area (namespace B-tree, per-file extents)  │
 ├──────────────────────────────────────────────────┤
 │  Data area (file content blocks)                  │
 └──────────────────────────────────────────────────┘
```

### On-disk types  (`helix/src/types.rs`)

```rust
/// Superblock — always at block 0.
#[repr(C)]
pub struct HelixSuperblock {
    pub magic:            [u8; 8],      // "HELIXFS\0"
    pub version:          u32,
    pub block_size:       u32,          // 4096
    pub total_blocks:     u64,
    pub free_blocks:      u64,
    pub committed_lsn:    u64,          // Highest flushed LSN
    pub log_head_segment: u64,
    pub log_head_offset:  u64,
    pub log_tail_segment: u64,
    pub root_inode_key:   u64,
    // ... additional fields ...
}

/// Stat result (returned by SYS_STAT via buf pointer).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileStat {
    pub size:        u64,
    pub created_ns:  u64,
    pub modified_ns: u64,
    pub is_dir:      bool,
    pub key:         u64,
}

/// Directory entry (returned by SYS_READDIR via buf pointer).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name:   [u8; 256],
    pub is_dir: bool,
    pub size:   u64,
    pub key:    u64,
}
```

### Log operations

Every mutation is represented as a `LogOp` appended to the write-ahead log:

```rust
pub enum LogOp {
    CreateFile,
    WriteExtent,
    DeleteFile,
    CreateDir,
    Rename,
    Snapshot,
    MkDir,
    Unlink,
    // etc.
}
```

### Block device abstraction

HelixFS is generic over `BlockIo`. In production it uses a RAM-backed device:

```rust
/// RAM-backed block device for the root filesystem.
pub struct MemBlockDevice {
    base: *mut u8,
    block_count: u64,
    block_size: u32,
}
```

Initialized during platform init Phase 11 with 16 MB from MemoryRegistry.

### Key design properties

- **No in-place overwrites**: All writes are log appends. The index is updated
  in-memory and periodically checkpointed.
- **Crash recovery**: Replay the log from the last committed LSN.
- **Time-travel reads**: Open a file with `O_AT_LSN` flag to read historical
  versions (when `SYS_VERSIONS` is fully implemented).
- **No fragmentation overhead**: The log is circular; old segments are reclaimed
  when the log wraps.

---

## 32. VFS — Virtual Filesystem Layer

**Crate**: `morpheus-helix`  **Path**: `helix/src/vfs/`

The VFS sits between the syscall handlers and the on-disk filesystem. It
manages mount points, per-process file descriptor tables, and delegates to
the appropriate filesystem instance.

### Mount table

```rust
pub struct MountTable {
    entries: [Option<MountEntry>; 8],
}

pub struct MountEntry {
    pub mount_point: [u8; 256],
    pub mount_point_len: u16,
    pub fs: FsInstance,
    pub read_only: bool,
}

impl MountTable {
    /// Mount a filesystem at the given path.
    pub fn mount(
        &mut self, mount_point: &str, instance: FsInstance, read_only: bool,
    ) -> Result<u8, HelixError>;

    /// Resolve a path to the best matching mount.
    pub fn resolve(&self, path: &str) -> Option<(u8, &MountEntry)>;
}
```

### FsInstance

An `FsInstance` bundles all the per-filesystem state:

```rust
pub struct FsInstance {
    pub sb:    HelixSuperblock,
    pub log:   LogEngine,
    pub index: NamespaceIndex,
    pub bitmap: BlockBitmap,
}
```

### File descriptor table (per-process)

```rust
pub const MAX_FDS: usize = 64;

pub struct FdTable {
    pub fds: [FileDescriptor; MAX_FDS],
}

pub struct FileDescriptor {
    pub flags:     u32,
    pub offset:    u64,
    pub mount_idx: u8,
    pub key:       u64,     // HelixFS index key for the file
    // ...
}
```

Each process in the scheduler has its own `FdTable`. File descriptors 0–2 are
reserved (stdin, stdout, stderr) and handled specially by the syscall handlers:

| fd | Target |
|----|--------|
| 0 | stdin (keyboard ring buffer) |
| 1 | stdout (COM1 serial) |
| 2 | stderr (COM1 serial) |
| 3–63 | VFS file descriptors |

### VFS API

All operations are called by the syscall handlers in `hwinit/src/syscall/handler.rs`:

```rust
// Open a file or directory, allocating an fd.
pub fn vfs_open(block_io, mount_table, fd_table, path, flags, timestamp) -> Result<usize, HelixError>;

// Read from an open fd into a buffer.
pub fn vfs_read(block_io, mount_table, fd_table, fd, buf) -> Result<usize, HelixError>;

// Write data to an open fd.
pub fn vfs_write(block_io, mount_table, fd_table, fd, data, timestamp) -> Result<usize, HelixError>;

// Seek within a file (SEEK_SET, SEEK_CUR, SEEK_END).
pub fn vfs_seek(mount_table, fd_table, fd, offset, whence) -> Result<u64, HelixError>;

// Close an fd.
pub fn vfs_close(fd_table, fd) -> Result<(), HelixError>;

// Stat a path (does not require an open fd).
pub fn vfs_stat(mount_table, path) -> Result<FileStat, HelixError>;

// List directory contents.
pub fn vfs_readdir(mount_table, path) -> Result<Vec<DirEntry>, HelixError>;

// Create a directory.
pub fn vfs_mkdir(mount_table, path, timestamp) -> Result<(), HelixError>;

// Delete a file or empty directory.
pub fn vfs_unlink(mount_table, path, timestamp) -> Result<(), HelixError>;

// Rename a file or directory.
pub fn vfs_rename(mount_table, old_path, new_path, timestamp) -> Result<(), HelixError>;

// Flush all pending log records and update superblock.
pub fn vfs_sync(block_io, mount_table) -> Result<(), HelixError>;
```

### Global filesystem singleton

```rust
/// Initialize the root filesystem (called from platform init Phase 11).
pub unsafe fn init_root_fs(device: MemBlockDevice, block_count: u64);

/// Get a read-only reference to the global FS state.
pub fn fs_global() -> Option<&'static FsGlobal>;

/// Get a mutable reference to the global FS state.
pub unsafe fn fs_global_mut() -> Option<&'static mut FsGlobal>;

pub struct FsGlobal {
    pub device:      MemBlockDevice,
    pub mount_table: MountTable,
}
```

### HelixError

```rust
pub enum HelixError {
    NotFound,
    AlreadyExists,
    InvalidFd,
    TooManyOpenFiles,
    ReadOnly,
    IsADirectory,
    DirectoryNotEmpty,
    NoSpace,
    MountNotFound,
    MountTableFull,
    PermissionDenied,
    InvalidOffset,
    IoReadFailed,
    IoWriteFailed,
    IoFlushFailed,
    // ...
}
```

---

## 33. Ring 3 User Processes

**Crate**: `morpheus-hwinit`

MorpheusX supports true Ring 3 user-mode execution with hardware-enforced
memory isolation. Each user process has:

1. **Own page table** — PML4 entries 0–255 (user-half, 128 TiB virtual)
   are private per process. Entries 256–511 (kernel-half) are cloned from
   the kernel page table so interrupts and syscalls work.
2. **Private kernel stack** — For Ring 3 → Ring 0 transitions (interrupts,
   syscalls). Set via TSS RSP0 and the `kernel_syscall_rsp` global.
3. **Per-process fd table** — 64 file descriptors, independent from other
   processes.

### Address space layout (user process)

```
 Virtual Address                        Description
 ─────────────────────────────────────────────────────
 0x0000_0000_0040_0000                  .text (ELF entry point)
 ...                                     .rodata, .data, .bss
 0x0000_007F_FFFF_7000                  User stack bottom (32 KiB, 8 pages)
 0x0000_007F_FFFF_F000                  User stack top (RSP initial value)
 ─────────────────────────────────────────────────────
 0xFFFF_8000_0000_0000+                 Kernel half (shared, no USER bit)
```

### SYSCALL/SYSRET mechanism

The x86-64 `SYSCALL` instruction performs a fast Ring 3 → Ring 0 transition:

1. User calls `syscall` with RAX=number, args in RDI/RSI/RDX/R10/R8/R9
2. CPU atomically: saves RIP to RCX, saves RFLAGS to R11, loads kernel
   CS/SS from `IA32_STAR`, jumps to `IA32_LSTAR` (our `syscall_entry`)
3. ASM trampoline: swaps to kernel stack, saves user registers, calls
   `syscall_dispatch()` in Rust
4. Return via `SYSRET`: restores user CS/SS/RIP/RFLAGS

### Segment selectors for SYSRET

```
IA32_STAR[47:32] = 0x08    (kernel CS base)
IA32_STAR[63:48] = 0x18    (user CS base — SYSRET adds 16 for CS, 8 for SS)

Result: kernel CS=0x08, SS=0x10, user CS=0x23 (0x20|3), SS=0x1B (0x18|3)
```

### Context switch with CR3

The timer ISR in `context_switch.s` loads `next_cr3` (written by
`scheduler_tick()`) into CR3 before restoring the next process's registers.
If the next process has the same CR3, the load is skipped.

```asm
; In irq_timer_isr:
mov rax, [next_cr3]    ; scheduler wrote this
mov rcx, cr3
cmp rax, rcx
je .skip_cr3           ; same address space — skip TLB flush
mov cr3, rax
.skip_cr3:
; ... restore GPRs, iretq
```

---

## 34. ELF Loader

**Crate**: `morpheus-hwinit`  **Path**: `hwinit/src/elf.rs`

The ELF loader parses ELF64 binaries and loads them into a fresh address space.

### API

```rust
/// Validate an ELF64 header (magic, class, endianness, arch, type).
pub fn validate_elf64(data: &[u8]) -> Result<&Elf64Ehdr, ElfError>;

/// Load an ELF64 binary into a new page table.
/// Returns the loaded image metadata and the PageTableManager.
pub unsafe fn load_elf64(data: &[u8]) -> Result<(ElfImage, PageTableManager), ElfError>;

pub struct ElfImage {
    pub entry:    u64,                // Entry point virtual address
    pub segments: Vec<LoadedSegment>, // Loaded PT_LOAD segments
}

pub struct LoadedSegment {
    pub vaddr: u64,      // Virtual address base (page-aligned)
    pub phys:  u64,      // Physical address base
    pub memsz: u64,      // Size in bytes (page-aligned)
    pub flags: PageFlags, // Page flags (USER, WRITABLE, NO_EXECUTE)
}
```

### Loading process

1. Validate ELF64 header (magic, x86-64, executable/shared type)
2. Allocate a new PML4 page and clone all 512 entries from kernel page table
3. For each `PT_LOAD` segment:
   - Allocate physical frames from MemoryRegistry
   - Zero the region, then copy file data (`.text`, `.rodata`, `.data`)
   - Map pages with USER bit through all 4 paging levels (PML4→PDPT→PD→PT)
   - Intermediate entries get PRESENT | WRITABLE | USER
4. Allocate and map user stack (8 pages = 32 KiB at `0x7FFFFFF7000..0x7FFFFFFFF000`)
5. Return `(ElfImage, PageTableManager)` — the scheduler uses `pml4_phys` as CR3

### ELF flags → page flags

| ELF flag | Page flags |
|----------|-----------|
| `PF_R` only | PRESENT \| USER \| NO_EXECUTE |
| `PF_R \| PF_W` | PRESENT \| USER \| WRITABLE \| NO_EXECUTE |
| `PF_R \| PF_X` | PRESENT \| USER |
| `PF_R \| PF_W \| PF_X` | PRESENT \| USER \| WRITABLE |

### Constants

```rust
pub const USER_STACK_PAGES: u64 = 8;     // 32 KiB
pub const USER_STACK_SIZE:  u64 = 32768; // 8 × 4096
pub const USER_STACK_TOP:   u64 = 0x0000_007F_FFFF_F000;
```

---

## 35. libmorpheus — Userspace SDK

**Crate**: `libmorpheus`  **Path**: `libmorpheus/`  
**Dependencies**: Zero. Pure inline ASM + thin wrappers.

This is the userspace library that Ring 3 binaries link against. It provides:

- `entry!(main)` macro for defining the ELF entry point
- Raw syscall wrappers (`syscall0`..`syscall5`)
- High-level file operations (`fs` module)
- Process management (`process` module)
- Console I/O (`io` module)

### Entry point (`libmorpheus::entry`)

```rust
#![no_std]
#![no_main]

use libmorpheus::entry;

entry!(main);

fn main() -> i32 {
    libmorpheus::io::println("Hello from Ring 3!");
    0
}
```

The `entry!` macro expands to:

```rust
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let code: i32 = main();
    libmorpheus::process::exit(code);
}
```

### Panic handler

The crate provides a `#[panic_handler]` that writes "PANIC in user process\n"
to stderr (fd 2) via `SYS_WRITE`, then calls `exit(101)`.

### Raw syscalls (`libmorpheus::raw`)

All syscall numbers are mirrored from `hwinit/src/syscall/mod.rs`:

```rust
pub const SYS_EXIT:     u64 = 0;
pub const SYS_WRITE:    u64 = 1;
// ... through SYS_VERSIONS = 21

pub unsafe fn syscall0(nr: u64) -> u64;
pub unsafe fn syscall1(nr: u64, a1: u64) -> u64;
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> u64;
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64;
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64;
pub unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64;
```

### File operations (`libmorpheus::fs`)

```rust
pub const O_READ:   u32 = 0x01;
pub const O_WRITE:  u32 = 0x02;
pub const O_CREATE: u32 = 0x04;
pub const O_TRUNC:  u32 = 0x10;
pub const O_APPEND: u32 = 0x20;

pub fn open(path: &str, flags: u32)           -> Result<usize, u64>;
pub fn read(fd: usize, buf: &mut [u8])        -> Result<usize, u64>;
pub fn write(fd: usize, data: &[u8])          -> Result<usize, u64>;
pub fn close(fd: usize)                       -> Result<(), u64>;
pub fn seek(fd: usize, offset: i64, whence: u64) -> Result<u64, u64>;
pub fn mkdir(path: &str)                      -> Result<(), u64>;
pub fn unlink(path: &str)                     -> Result<(), u64>;
pub fn rename(old: &str, new: &str)           -> Result<(), u64>;
pub fn stat(path: &str, buf: &mut [u8])       -> Result<(), u64>;
pub fn sync()                                 -> Result<(), u64>;
```

### Process management (`libmorpheus::process`)

```rust
pub fn exit(code: i32) -> !;
pub fn getpid() -> u32;
pub fn yield_cpu();
pub fn kill(pid: u32, signal: u8) -> Result<(), u64>;
pub fn sleep(millis: u64);
```

> **Note**: `sleep()` takes **milliseconds**. The kernel computes a TSC
> deadline internally.

### Console I/O (`libmorpheus::io`)

```rust
pub fn print(s: &str);     // Write to stdout (fd 1 → serial)
pub fn println(s: &str);   // print(s) + print("\n")
```

### Error checking

```rust
/// Returns true if a syscall return value represents an error.
/// Errors are in the range (u64::MAX - 255)..=u64::MAX.
pub fn is_error(ret: u64) -> bool;
```

### Complete example — file I/O from userspace

```rust
#![no_std]
#![no_main]

use libmorpheus::entry;
use libmorpheus::fs;
use libmorpheus::io;
use libmorpheus::process;

entry!(main);

fn main() -> i32 {
    io::println("Creating a file...");

    // Create and write
    let fd = match fs::open("/tmp/hello.txt", fs::O_WRITE | fs::O_CREATE) {
        Ok(fd) => fd,
        Err(_) => { io::println("open failed"); return 1; }
    };
    let _ = fs::write(fd, b"Hello from Ring 3!\n");
    let _ = fs::close(fd);

    // Read back
    let fd = match fs::open("/tmp/hello.txt", fs::O_READ) {
        Ok(fd) => fd,
        Err(_) => { io::println("read open failed"); return 1; }
    };
    let mut buf = [0u8; 128];
    match fs::read(fd, &mut buf) {
        Ok(n) => {
            if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                io::print("Read: ");
                io::println(s);
            }
        }
        Err(_) => io::println("read failed"),
    }
    let _ = fs::close(fd);

    io::println("Done!");
    0
}
```

---

## 36. Building Userspace Binaries

### Custom target: `x86_64-morpheus.json`

User processes are ELF64 static binaries built with a custom target spec:

```json
{
  "llvm-target": "x86_64-unknown-none",
  "arch": "x86_64",
  "os": "none",
  "executables": true,
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "disable-redzone": true,
  "features": "-mmx,-sse,-sse2,+soft-float",
  "relocation-model": "static",
  "code-model": "small",
  "pre-link-args": {
    "ld.lld": ["-Tlibmorpheus/linker.ld", "--gc-sections"]
  },
  "max-atomic-width": 64
}
```

Key properties:
- **No red zone** (required for interrupt safety)
- **Soft-float** (kernel never initializes SSE/FPU state)
- **Static linking** (no dynamic loader)
- **Linker script** places `.text` at `0x400000`

### Linker script: `libmorpheus/linker.ld`

```ld
OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS {
    . = 0x400000;
    .text   ALIGN(4K) : { *(.text._start) *(.text .text.*) }
    .rodata ALIGN(4K) : { *(.rodata .rodata.*) }
    .data   ALIGN(4K) : { *(.data .data.*) }
    .bss    ALIGN(4K) : { __bss_start = .; *(.bss .bss.*) *(COMMON) __bss_end = .; }
    /DISCARD/ : { *(.comment) *(.note.*) *(.eh_frame*) *(.debug_*) }
}
```

### Build commands

```bash
# Build a userspace binary
cargo build --release \
  --target x86_64-morpheus.json \
  -p my-user-app

# The output is an ELF64 binary at:
# target/x86_64-morpheus/release/my-user-app
```

### Deploying to the filesystem

Place the compiled ELF binary in the HelixFS root at `/bin/<name>`:

```rust
// From a Ring 0 context (e.g., during platform init or a kernel app):
let binary = include_bytes!("path/to/my-user-app");
morpheus_helix::ops::write::write_file(
    block_io, &mut fs.log, &mut fs.index, &fs.bitmap,
    "/bin/hello", binary, timestamp,
)?;
```

Then from the shell:

```
morpheus> exec hello
Spawned 'hello' as PID 3
```

### Cargo.toml for a userspace crate

```toml
[package]
name = "my-user-app"
version = "0.1.0"
edition = "2021"

[dependencies]
libmorpheus = { path = "../libmorpheus" }

[profile.release]
panic = "abort"
opt-level = "s"    # Optimize for size
lto = true
```

### `src/main.rs`

```rust
#![no_std]
#![no_main]

use libmorpheus::entry;

entry!(main);

fn main() -> i32 {
    libmorpheus::io::println("hello world");
    0
}
```

---

## 37. stdin — Keyboard Input Buffer

**Crate**: `morpheus-hwinit`  **Path**: `hwinit/src/stdin.rs`

A lock-free SPSC (single-producer / single-consumer) ring buffer that connects
the desktop keyboard handler to user processes reading from fd 0.

### Architecture

```
 Keyboard ISR → desktop event loop → stdin::push(byte)
                                          │
                                          ▼
                                    ┌──────────┐
                                    │  256-byte │
                                    │ ring buf  │
                                    └──────────┘
                                          │
                                          ▼
                     SYS_READ(fd=0) → stdin::read(buf) → user process
```

### API

```rust
/// Push a single ASCII byte into the stdin buffer.
/// Called by the desktop event loop for each printable keypress.
/// Returns false if the buffer is full (byte dropped).
pub fn push(byte: u8) -> bool;

/// Read up to buf.len() bytes from stdin.
/// Returns immediately with 0 if the buffer is empty.
pub fn read(buf: &mut [u8]) -> usize;

/// Number of unread bytes available.
pub fn available() -> usize;
```

### Buffer properties

- **Size**: 256 bytes (power of two for efficient masking)
- **Ordering**: Atomic `Acquire`/`Release` on head/tail indices
- **Overflow**: Bytes are silently dropped when full
- **Non-blocking**: `read()` returns 0 immediately if empty

### How keyboard input flows

1. `keyboard.poll_key_with_delay()` detects a keypress in the desktop event loop
2. If `unicode_char > 0 && unicode_char < 128`, the byte is pushed to `stdin::push()`
3. The same event is also translated and dispatched to the focused Ring 0 app/shell
4. A Ring 3 process calls `SYS_READ(fd=0, buf, len)` → kernel calls `stdin::read(buf)`
5. The kernel returns however many bytes were available (0 if none)

---

## 38. Platform Capability Matrix

This section maps what can and cannot be built on MorpheusX today, based on the
available kernel primitives, drivers, and syscall surface.

### What you CAN build today

| Application | Required primitives | Status |
|------------|-------------------|--------|
| **File manager** | VFS (open/read/write/close/readdir/stat/mkdir/unlink/rename), Canvas, Widgets (List, Panel, Button) | ✅ All present |
| **Text editor** | VFS read/write, TextArea, TextInput, keyboard events | ✅ All present |
| **Process monitor / Task manager** | SCHEDULER.snapshot_processes(), send_signal(), Canvas, List, ProgressBar | ✅ Already implemented (`open tasks`) |
| **System info viewer** | MemoryRegistry stats, heap_stats(), PCI enumeration, TSC frequency | ✅ All present |
| **Simple HTTP client** | NetInterface (internal), TCP connect/send/recv, DNS | ⚠️ Internal only; not exposed to apps via syscall |
| **Serial terminal** | SYS_READ(0) for stdin, SYS_WRITE(1) for stdout | ✅ Ring 3 capable |
| **CLI utilities** | libmorpheus (fs, process, io), stdin, stdout | ✅ Ring 3 capable |
| **Games (text-mode)** | Canvas, draw primitives, keyboard events, timer ticks | ✅ Ring 0 apps |
| **Calculator** | Canvas, TextInput, Button, Label | ✅ Ring 0 apps |
| **Hex viewer / Binary inspector** | VFS read, TextArea, scroll | ✅ All present |

### What CANNOT be built today (gaps)

| Application | Missing primitive | Severity | Notes |
|------------|------------------|----------|-------|
| **Web browser** | TLS/HTTPS, HTML/CSS parser, image decoder, Unicode font engine, general TCP socket API | 🔴 CRITICAL | Multiple foundational gaps |
| **HTTPS client** | TLS library (e.g., rustls or custom) | 🔴 CRITICAL | `HttpsNotSupported` error exists in codebase |
| **Image viewer** | PNG/JPEG decoder, image→Canvas blit | 🟡 HIGH | Could implement with pure-Rust decoders |
| **Rich text / Unicode** | Unicode line breaking, glyph shaping, TrueType/OpenType renderer | 🟡 HIGH | Current font is 8×16 CP437 bitmap only |
| **Network app (user-space)** | SYS_SOCKET / SYS_CONNECT / SYS_SENDTO / SYS_RECVFROM | 🟡 HIGH | NetInterface exists but has no syscall exposure |
| **Pipe-based shell** | SYS_PIPE, SYS_DUP, SYS_POLL/SELECT | 🟠 MEDIUM | No IPC beyond signals |
| **Memory-mapped files** | SYS_MMAP | 🟠 MEDIUM | Only SYS_ALLOC (physical pages) |
| **Clipboard / Copy-paste** | Shared memory or clipboard syscall | 🟢 LOW | Could be added easily |
| **GPU-accelerated rendering** | GPU driver, DRM/KMS-like API | 🟢 LOW | Software rendering only |

### Network stack inventory

| Protocol | Layer | Implementation | Accessible to apps? |
|----------|-------|---------------|-------------------|
| Ethernet (VirtIO-net) | L2 | ✅ Full driver | No — kernel internal |
| Ethernet (e1000e) | L2 | ✅ Full driver | No — kernel internal |
| ARP | L2.5 | ✅ Cache + resolution | No — kernel internal |
| IPv4 | L3 | ✅ Full | No — kernel internal |
| TCP | L4 | ✅ Full (via smoltcp) | No — kernel internal |
| UDP | L4 | ✅ Internal (DHCP/DNS) | No — kernel internal |
| DHCP | App | ✅ Auto-configuration | No — kernel internal |
| DNS | App | ✅ A record resolution | No — kernel internal |
| HTTP/1.1 | App | ✅ GET with streaming | No — kernel internal |
| TLS/HTTPS | App | ❌ Not implemented | N/A |
| IPv6 | L3 | ❌ Not implemented | N/A |

### Roadmap priorities for higher-level applications

1. **Expose TCP sockets** via new syscalls (`SYS_SOCKET`, `SYS_CONNECT`,
   `SYS_SEND`, `SYS_RECV`, `SYS_BIND`, `SYS_LISTEN`, `SYS_ACCEPT`)
   — this unblocks all network-capable user applications.
2. **TLS library** — port a `no_std` TLS implementation (e.g., `rustls` with
   ring's crypto) to enable HTTPS. Requires step 1 first.
3. **Image decoders** — port `png` and `jpeg-decoder` crates (`no_std` mode)
   for image viewing.
4. **Unicode font engine** — implement a TrueType rasterizer or integrate a
   `no_std` bitmap Unicode font for international text support.
5. **Pipe / IPC** — add `SYS_PIPE` and `SYS_POLL` for shell pipelines and
   inter-process communication.

---

*End of MorpheusX SDK Reference*
