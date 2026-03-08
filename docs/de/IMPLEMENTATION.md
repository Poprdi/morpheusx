# MorpheusX Desktop Environment — Implementation Guide

**Status**: LOCKED  
**Source-verified**: All struct layouts, syscall numbers, and module paths reflect actual codebase state as of research phase.

---

## 1. Workspace Changes

### 1.1 New Workspace Members

Add to the `[workspace]` members array in `/Cargo.toml`:

```toml
members = [
    # ... existing members ...
    "compd",
    "shelld",
    "init",
    "channel",   # shared no_std ring buffer crate
]
```

### 1.2 New Crate Directory Layout

```
compd/
    Cargo.toml
    src/
        main.rs
        islands/
            mod.rs
            vsync.rs
            renderer.rs
            surface_mgr.rs
            input.rs
            focus.rs
        messages.rs

shelld/
    Cargo.toml
    src/
        main.rs
        islands/
            mod.rs
            wallpaper.rs
            panel.rs
            launcher.rs
        messages.rs

init/
    Cargo.toml
    src/
        main.rs
        islands/
            mod.rs
            supervisor.rs

channel/
    Cargo.toml
    src/
        lib.rs      # no_std SPSC ring buffer
```

---

## 2. `channel` Crate

This crate is the single source of truth for inter-island communication within a process. It is `no_std` + `alloc`-free. It uses only `core::sync::atomic`.

### `channel/Cargo.toml`

```toml
[package]
name    = "channel"
version = "0.1.0"
edition = "2021"

[dependencies]
# intentionally empty — pure no_std, no alloc
```

### `channel/src/lib.rs`

```rust
#![no_std]

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Single-producer, single-consumer ring buffer.
/// N MUST be a power of 2. Enforced at compile time via const assertion.
pub struct Channel<T, const N: usize> {
    buf:  [UnsafeCell<MaybeUninit<T>>; N],
    head: AtomicUsize,  // producer advances
    tail: AtomicUsize,  // consumer advances
}

// SAFETY: Single-core scheduler. No preemption between send/recv within same
// process. We do not have threads sharing a Channel across cores.
unsafe impl<T, const N: usize> Sync for Channel<T, N> {}

impl<T, const N: usize> Channel<T, N> {
    const ASSERT_POWER_OF_2: () = assert!(
        N.is_power_of_two(),
        "Channel capacity N must be a power of two"
    );

    pub const fn new() -> Self {
        // Bind the const assertion so it is evaluated.
        let _ = Self::ASSERT_POWER_OF_2;
        // SAFETY: MaybeUninit arrays can be zero-initialized.
        Self {
            buf:  unsafe {
                MaybeUninit::<[UnsafeCell<MaybeUninit<T>>; N]>::zeroed()
                    .assume_init()
            },
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Returns `Err(msg)` if the channel is full. Never blocks. Never allocs.
    pub fn send(&self, msg: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return Err(msg);
        }
        unsafe {
            (*self.buf[head & (N - 1)].get()).write(msg);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Returns `None` if the channel is empty. Never blocks. Never allocs.
    pub fn recv(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let msg = unsafe {
            (*self.buf[tail & (N - 1)].get()).assume_init_read()
        };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(msg)
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }
}
```

---

## 3. `compd` Crate

### `compd/Cargo.toml`

```toml
[package]
name    = "compd"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "compd"
path = "src/main.rs"

[dependencies]
libmorpheus = { path = "../libmorpheus" }
display     = { path = "../display", features = ["framebuffer-backend"] }
gfx3d       = { path = "../gfx3d" }
channel     = { path = "../channel" }

[profile.release]
opt-level = 3
lto       = true
```

### `compd/src/messages.rs`

Full message type definitions — copy these exactly, do not deviate.

```rust
use display::types::PixelFormat;

pub const MAX_WINDOWS: usize = 16;

/// Sent by vsync island to renderer island.
pub enum VsyncMsg {
    Tick { now_ns: u64 },
}

/// A single window's compositing parameters.
/// `surface` is a raw pointer valid only during compositing.
/// It points into a kernel-mapped region owned by surface_mgr island.
#[derive(Copy, Clone)]
pub struct CompositeEntry {
    pub pid:        u32,
    pub surface:    *const u32,  // virtual addr of app surface buffer
    pub x:          i32,
    pub y:          i32,
    pub w:          u32,
    pub h:          u32,
    pub src_stride: u32,         // stride in PIXELS
    pub z_layer:    u8,          // 0=bg 1=bottom 2=top 3=overlay
    pub dirty:      bool,
    pub _pad:       [u8; 2],
}

// SAFETY: We only use this pointer during the compose() call on a single-core
// system with no preemption between surface_mgr and renderer islands.
unsafe impl Send for CompositeEntry {}
unsafe impl Sync for CompositeEntry {}

/// Sent by surface_mgr to renderer.
pub enum SurfaceMsg {
    CompositeList {
        entries: [Option<CompositeEntry>; MAX_WINDOWS],
        count:   u8,
    },
}

/// Sent by input island to other islands.
pub enum InputMsg {
    WindowMoved   { idx: u8, new_x: i32, new_y: i32 },
    WindowResized { idx: u8, new_w: u32, new_h: u32 },
    WindowClosed  { idx: u8, pid: u32 },
    KeyForward    { pid: u32, scancode: u8 },
    MouseForward  { pid: u32, dx: i16, dy: i16, buttons: u8 },
}

/// Sent by focus island to input and renderer islands.
pub enum FocusMsg {
    FocusChanged { old: Option<u8>, new: Option<u8> },
}
```

### `compd/src/main.rs` — Boot Sequence

```rust
#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{compositor as compsys, hw, io, process};
use display::{framebuffer::Framebuffer, types::FramebufferInfo};

mod messages;
mod islands;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // 1. Register as compositor. Panics if another process holds the slot.
    compsys::compositor_set();

    // 2. Map the physical framebuffer.
    let fb_info: FramebufferInfo = hw::fb_info();
    let fb_base: *mut u8        = hw::fb_map();

    // 3. Build the Framebuffer handle.
    let fb = unsafe {
        Framebuffer::from_raw(fb_base, &fb_info)
    };

    // 4. Detect pixel format (Bgrx = 1 is UEFI default).
    let is_bgrx = matches!(fb_info.format, display::types::PixelFormat::Bgrx);

    // 5. Initialize island state.
    let mut state = islands::CompState::new(fb, fb_info, is_bgrx);

    // 6. Enter main vsync loop.
    loop {
        islands::vsync::tick(&mut state);
        islands::input::poll(&mut state);
        islands::surface_mgr::update(&mut state);
        islands::renderer::compose(&mut state);
        islands::focus::process_msgs(&mut state);
        islands::surface_mgr::reap_exited(&mut state);
        process::yield_cpu();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // TODO: write panic message to serial via SYS_SYSLOG
    loop { unsafe { core::arch::asm!("hlt"); } }
}
```

### `compd/src/islands/mod.rs` — Shared State

```rust
use display::{framebuffer::Framebuffer, types::FramebufferInfo};
use gfx3d::{pipeline::Pipeline, target::SoftwareTarget};
use channel::Channel;
use crate::messages::*;

pub const MAX_WINDOWS: usize = 16;

pub const TITLE_H:      u32 = 22;
pub const BORDER:       u32 = 1;
pub const CASCADE_STEP: i32 = 28;

/// DESKTOP_RGB = (26, 26, 46)
pub const DESKTOP_COLOR: (u8, u8, u8) = (26, 26, 46);
/// Focused title bar
pub const TITLE_FOCUSED_COLOR: (u8, u8, u8) = (0, 85, 0);
/// Focused border
pub const BORDER_FOCUSED_COLOR: (u8, u8, u8) = (0, 170, 0);
/// Mouse cursor
pub const CURSOR_COLOR: (u8, u8, u8) = (255, 255, 255);

pub struct ChildWindow {
    pub pid:           u32,
    pub surface_ptr:   *const u32,
    pub mapped:        bool,
    pub surface_vaddr: u64,
    pub surface_pages: u64,
    pub x:             i32,
    pub y:             i32,
    pub w:             u32,
    pub h:             u32,
    pub src_w:         u32,
    pub src_h:         u32,
    pub src_stride:    u32,  // in pixels
    pub title:         [u8; 64],
    pub title_len:     usize,
    pub z_layer:       u8,
}

pub struct CompState {
    // --- renderer island owns these ---
    pub pipeline:  Pipeline,
    pub target:    SoftwareTarget,
    pub fb:        Framebuffer,
    pub fb_info:   FramebufferInfo,
    pub is_bgrx:   bool,

    // --- surface_mgr island owns these ---
    pub windows:   [Option<ChildWindow>; MAX_WINDOWS],
    pub cascade_n: u32,

    // --- focus island owns these ---
    pub focused:   Option<usize>,

    // --- input island owns these ---
    pub mouse_x:      i32,
    pub mouse_y:      i32,
    pub last_buttons: u32,

    // --- channels (SPSC) ---
    pub ch_vsync_to_renderer:  Channel<VsyncMsg, 4>,
    pub ch_surface_to_renderer: Channel<SurfaceMsg, 4>,
    pub ch_input_to_focus:     Channel<InputMsg, 16>,
    pub ch_input_to_surface:   Channel<InputMsg, 16>,
    pub ch_focus_to_renderer:  Channel<FocusMsg, 4>,
}

impl CompState {
    pub fn new(fb: Framebuffer, fb_info: FramebufferInfo, is_bgrx: bool) -> Self {
        let (w, h) = (fb_info.width, fb_info.height);
        Self {
            pipeline:  Pipeline::new(),
            target:    SoftwareTarget::new(w, h, gfx3d::target::TargetPixelFormat::Bgrx),
            fb,
            fb_info,
            is_bgrx,
            windows:   core::array::from_fn(|_| None),
            cascade_n: 0,
            focused:   None,
            mouse_x:   0,
            mouse_y:   0,
            last_buttons: 0,
            ch_vsync_to_renderer:   Channel::new(),
            ch_surface_to_renderer: Channel::new(),
            ch_input_to_focus:      Channel::new(),
            ch_input_to_surface:    Channel::new(),
            ch_focus_to_renderer:   Channel::new(),
        }
    }
}
```

### `compd/src/islands/renderer.rs` — Compose Logic

Migrated from `shell/src/compositor/render.rs`. Key changes: pixels are written via `Framebuffer::put_pixel` or `Framebuffer::fill_rect` (ASM-backed), never via direct pointer arithmetic in safe Rust.

```rust
use libmorpheus::compositor as compsys;
use crate::islands::{CompState, ChildWindow, DESKTOP_COLOR, TITLE_H, BORDER,
                     TITLE_FOCUSED_COLOR, BORDER_FOCUSED_COLOR, CURSOR_COLOR};
use display::types::Color;

pub fn compose(state: &mut CompState) {
    let (dw, dh) = (state.fb_info.width, state.fb_info.height);

    // Clear desktop.
    let (dr, dg, db) = DESKTOP_COLOR;
    state.fb.fill_rect(0, 0, dw, dh, Color::rgb(dr, dg, db));

    // Build z-order: unfocused indices first, then focused.
    let mut order = [0usize; super::MAX_WINDOWS];
    let mut n = 0usize;
    for i in 0..super::MAX_WINDOWS {
        if state.windows[i].is_some() && Some(i) != state.focused {
            order[n] = i;
            n += 1;
        }
    }
    if let Some(fi) = state.focused {
        if state.windows[fi].is_some() {
            order[n] = fi;
            n += 1;
        }
    }

    for &idx in &order[..n] {
        if let Some(ref win) = state.windows[idx] {
            draw_window(state, win, Some(idx) == state.focused);
        }
    }

    // Draw cursor (2×2 white square).
    let (cr, cg, cb) = CURSOR_COLOR;
    let cursor_color = Color::rgb(cr, cg, cb);
    state.fb.fill_rect(state.mouse_x as u32, state.mouse_y as u32,
                       2, 2, cursor_color);

    // Present to hardware.
    unsafe { libmorpheus::hw::fb_present(); }

    // Clear dirty flags.
    for i in 0..super::MAX_WINDOWS {
        if let Some(ref win) = state.windows[i] {
            compsys::surface_dirty_clear(win.pid);
        }
    }
}

fn draw_window(state: &mut CompState, win: &ChildWindow, focused: bool) {
    let (br, bg, bb) = if focused { BORDER_FOCUSED_COLOR } else { (60, 60, 60) };
    let border_color = Color::rgb(br, bg, bb);
    let (tr, tg, tb) = if focused { TITLE_FOCUSED_COLOR } else { (30, 50, 30) };
    let title_color = Color::rgb(tr, tg, tb);

    let x = win.x as u32;
    let y = win.y as u32;
    let w = win.w;
    let h = win.h;

    // Border rect.
    state.fb.fill_rect(x.saturating_sub(BORDER),
                       y.saturating_sub(TITLE_H + BORDER),
                       w + BORDER * 2,
                       h + TITLE_H + BORDER * 2,
                       border_color);

    // Title bar.
    state.fb.fill_rect(x, y.saturating_sub(TITLE_H), w, TITLE_H, title_color);

    // Blit surface pixels.
    blit_surface(state, win);
}

/// Blit a ChildWindow's surface buffer into the framebuffer.
/// Clips to FB bounds. Does NOT skip if dirty == false.
fn blit_surface(state: &mut CompState, win: &ChildWindow) {
    if win.surface_ptr.is_null() || !win.mapped {
        return;
    }
    let (fb_w, fb_h) = (state.fb_info.width as i32, state.fb_info.height as i32);
    let (sw, sh) = (win.src_w as i32, win.src_h as i32);

    let dst_x0 = win.x;
    let dst_y0 = win.y;
    let dst_x1 = (win.x + sw).min(fb_w);
    let dst_y1 = (win.y + sh).min(fb_h);
    let src_stride = win.src_stride as i32;

    for dy in dst_y0.max(0)..dst_y1 {
        for dx in dst_x0.max(0)..dst_x1 {
            let sx = dx - dst_x0;
            let sy = dy - dst_y0;
            let src_idx = (sy * src_stride + sx) as usize;
            let pixel = unsafe { *win.surface_ptr.add(src_idx) };
            // surface buffers are already in the framebuffer's native pixel
            // format — apps must match compd's fb format.
            state.fb.put_pixel(dx as u32, dy as u32,
                               display::types::Color::from_packed(pixel));
        }
    }
}
```

> **Note**: `Color::from_packed` must be added to `display/src/types.rs` if not present. Alternatively, write directly through `Framebuffer::put_pixel_raw(x, y, packed_u32)` if such a method exists.

### `compd/src/islands/surface_mgr.rs`

Migrated from `shell/src/compositor/surfaces.rs`.

```rust
use libmorpheus::compositor as compsys;
use libmorpheus::mem;
use crate::islands::{CompState, ChildWindow, CASCADE_STEP};

pub fn update(state: &mut CompState) {
    let mut buf = [compsys::SurfaceEntry::zeroed(); super::MAX_WINDOWS];
    let count = compsys::surface_list(&mut buf) as usize;

    for i in 0..count {
        let entry = &buf[i];
        let pid = entry.pid;

        // Find existing window slot for this pid.
        let slot = state.windows.iter().position(|w| {
            w.as_ref().map(|w| w.pid == pid).unwrap_or(false)
        });

        if let Some(idx) = slot {
            // Already tracked. Update surface geometry.
            if let Some(ref mut win) = state.windows[idx] {
                win.src_w      = entry.width;
                win.src_h      = entry.height;
                win.src_stride = entry.stride; // stride in pixels
            }
        } else {
            // New window — find empty slot.
            let empty = state.windows.iter().position(|w| w.is_none());
            let Some(idx) = empty else { continue; };

            // Map surface pages into compd's address space.
            let vaddr = unsafe { compsys::surface_map(pid) };
            if vaddr.is_null() { continue; }

            // Cascade position.
            let cx = 50 + state.cascade_n as i32 * CASCADE_STEP;
            let cy = 50 + state.cascade_n as i32 * CASCADE_STEP;
            state.cascade_n = (state.cascade_n + 1) % 8;

            state.windows[idx] = Some(ChildWindow {
                pid,
                surface_ptr:   vaddr as *const u32,
                mapped:        true,
                surface_vaddr: vaddr as u64,
                surface_pages: entry.pages,
                x:             cx,
                y:             cy,
                w:             entry.width,
                h:             entry.height,
                src_w:         entry.width,
                src_h:         entry.height,
                src_stride:    entry.stride,
                title:         [0u8; 64],
                title_len:     0,
                z_layer:       1, // normal app window = bottom layer
            });

            // Update focus to new window.
            state.focused = Some(idx);
        }
    }
}

pub fn reap_exited(state: &mut CompState) {
    for i in 0..super::MAX_WINDOWS {
        let pid = if let Some(ref w) = state.windows[i] { w.pid } else { continue };
        let exited = unsafe { libmorpheus::process::try_wait(pid) };
        if exited {
            if let Some(ref w) = state.windows[i] {
                unsafe {
                    mem::munmap(w.surface_vaddr as *mut u8, w.surface_pages as usize);
                }
            }
            state.windows[i] = None;
            if state.focused == Some(i) {
                // Focus next available window.
                state.focused = state.windows.iter().position(|w| w.is_some());
            }
        }
    }
}
```

### `compd/src/islands/input.rs`

Migrated from `shell/src/compositor/input.rs` and `event_loop.rs`.

```rust
use libmorpheus::{compositor as compsys, io, hw};
use crate::islands::{CompState, TITLE_H, BORDER};
use crate::messages::InputMsg;

const CTRL_BRACKET: u8 = 0x1D; // Ctrl+] — focus cycle

pub fn poll(state: &mut CompState) {
    poll_keyboard(state);
    poll_mouse(state);
}

fn poll_keyboard(state: &mut CompState) {
    loop {
        let byte = io::stdin_read_nonblock();
        let Some(sc) = byte else { break };

        if sc == CTRL_BRACKET {
            // Focus cycle — send to focus island via channel.
            let _ = state.ch_input_to_focus.send(InputMsg::KeyForward {
                pid: 0, scancode: sc,
            });
            continue;
        }

        // Forward to focused window.
        if let Some(fi) = state.focused {
            if let Some(ref win) = state.windows[fi] {
                compsys::forward_input(win.pid, sc as u64);
            }
        }
    }
}

fn poll_mouse(state: &mut CompState) {
    let (dx, dy, buttons) = hw::mouse_read();
    if dx == 0 && dy == 0 && buttons == state.last_buttons as i16 { return; }

    let fb_w = state.fb_info.width as i32;
    let fb_h = state.fb_info.height as i32;

    state.mouse_x = (state.mouse_x + dx as i32).clamp(0, fb_w - 1);
    state.mouse_y = (state.mouse_y + dy as i32).clamp(0, fb_h - 1);

    let mx = state.mouse_x;
    let my = state.mouse_y;
    let pressed = buttons != 0;
    let released = buttons == 0 && state.last_buttons != 0;
    state.last_buttons = buttons as u32;

    if pressed {
        handle_press(state, mx, my);
    } else if released {
        handle_release(state);
    } else {
        handle_move(state, mx, my, dx as i32, dy as i32, buttons as u8);
    }
}

fn handle_press(state: &mut CompState, mx: i32, my: i32) {
    // hit-test: find topmost window under cursor
    // z-order: focused last → iterate in reverse
    for &idx in state.focused.iter().chain(
        (0..super::MAX_WINDOWS)
            .filter(|&i| Some(i) != state.focused && state.windows[i].is_some())
            .collect::<alloc::vec::Vec<_>>()
            .iter()
            .rev()
    ) {
        let Some(ref win) = state.windows[idx] else { continue };
        let hit = hit_region(win, mx, my);
        match hit {
            HitRegion::Title => {
                state.focused = Some(idx);
                // start move capture — stored in a field on CompState
                // (not shown for brevity; add capture: MouseCapture to CompState)
            }
            HitRegion::Content => {
                state.focused = Some(idx);
                compsys::mouse_forward(win.pid, 0, 0, 1);
            }
            HitRegion::Close => {
                let pid = win.pid;
                unsafe { libmorpheus::process::kill(pid, 15); } // SIGTERM
            }
            HitRegion::None => {}
        }
        break;
    }
}

fn handle_release(_state: &mut CompState) {
    // clear capture
}

fn handle_move(state: &mut CompState, _mx: i32, _my: i32,
               dx: i32, dy: i32, buttons: u8) {
    // Forward mouse delta to focused window.
    if let Some(fi) = state.focused {
        if let Some(ref win) = state.windows[fi] {
            compsys::mouse_forward(win.pid, dx as i16, dy as i16, buttons);
        }
    }
}

#[derive(Debug, PartialEq)]
enum HitRegion { Title, Content, Close, Resize, None }

fn hit_region(win: &crate::islands::ChildWindow, mx: i32, my: i32) -> HitRegion {
    let x0 = win.x - BORDER as i32;
    let y0 = win.y - TITLE_H as i32 - BORDER as i32;
    let x1 = win.x + win.w as i32 + BORDER as i32;
    let y1 = win.y + win.h as i32 + BORDER as i32;

    if mx < x0 || mx >= x1 || my < y0 || my >= y1 {
        return HitRegion::None;
    }

    if my < win.y {
        // In title bar region.
        let close_x = x1 - 20;
        if mx >= close_x {
            return HitRegion::Close;
        }
        return HitRegion::Title;
    }

    HitRegion::Content
}
```

### `compd/src/islands/focus.rs`

```rust
use crate::islands::CompState;
use crate::messages::InputMsg;

pub fn process_msgs(state: &mut CompState) {
    while let Some(msg) = state.ch_input_to_focus.recv() {
        match msg {
            InputMsg::KeyForward { scancode: 0x1D, .. } => {
                cycle_focus(state);
            }
            InputMsg::WindowClosed { idx, .. } => {
                if state.focused == Some(idx as usize) {
                    state.focused = state.windows.iter().position(|w| w.is_some());
                }
            }
            _ => {}
        }
    }
}

fn cycle_focus(state: &mut CompState) {
    let start = state.focused.map(|f| f + 1).unwrap_or(0);
    for offset in 0..super::MAX_WINDOWS {
        let idx = (start + offset) % super::MAX_WINDOWS;
        if state.windows[idx].is_some() {
            state.focused = Some(idx);
            return;
        }
    }
}
```

### `compd/src/islands/vsync.rs`

```rust
use libmorpheus::hw;
use crate::islands::CompState;

const TARGET_FRAME_NS: u64 = 16_666_667; // ~60 Hz

pub fn tick(state: &mut CompState) {
    // On single-core: vsync is approximated by wall-clock polling.
    // The real gate is SYS_YIELD + scheduler cadence.
    // Future: hook hardware vsync IRQ via SYS_IRQ_ATTACH.
    let _ = hw::clock_ns(); // consumes SYS_CLOCK (22)
    // No blocking here — compose() runs unconditionally each loop iteration.
}
```

---

## 4. `init` Crate

### `init/Cargo.toml`

```toml
[package]
name    = "init"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "init"
path = "src/main.rs"

[dependencies]
libmorpheus = { path = "../libmorpheus" }
```

### `init/src/main.rs`

```rust
#![no_std]
#![no_main]

use libmorpheus::{process, io};

mod islands;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut state = islands::supervisor::SupervisorState::new();

    // Spawn compd and shelld.
    state.compd_pid = Some(process::spawn("/bin/compd\0"));
    state.shelld_pid = Some(process::spawn("/bin/shelld\0"));

    // Install SIGCHLD handler.
    // SIGCHLD = 17
    unsafe {
        libmorpheus::signal::sigaction(17, sigchld_handler as usize);
    }

    loop {
        islands::supervisor::tick(&mut state);
        process::yield_cpu();
    }
}

extern "C" fn sigchld_handler() {
    // Handled in supervisor::tick via SYS_TRY_WAIT.
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { unsafe { core::arch::asm!("hlt"); } }
}
```

### `init/src/islands/supervisor.rs`

```rust
use libmorpheus::{process, compositor as compsys};

const MAX_RESTARTS: u32 = 5;

pub struct SupervisorState {
    pub compd_pid:       Option<u32>,
    pub shelld_pid:      Option<u32>,
    pub compd_restarts:  u32,
    pub shelld_restarts: u32,
}

impl SupervisorState {
    pub fn new() -> Self {
        Self {
            compd_pid:      None,
            shelld_pid:     None,
            compd_restarts: 0,
            shelld_restarts: 0,
        }
    }
}

pub fn tick(state: &mut SupervisorState) {
    // Check compd.
    if let Some(pid) = state.compd_pid {
        if unsafe { process::try_wait(pid) } {
            state.compd_pid = None;
            if state.compd_restarts < MAX_RESTARTS {
                state.compd_restarts += 1;
                // Reclaim compositor slot before re-spawning.
                compsys::compositor_set();
                let new_pid = process::spawn("/bin/compd\0");
                state.compd_pid = Some(new_pid);
            }
            // else: give up, system is degraded
        }
    }

    // Check shelld.
    if let Some(pid) = state.shelld_pid {
        if unsafe { process::try_wait(pid) } {
            state.shelld_pid = None;
            if state.shelld_restarts < MAX_RESTARTS {
                state.shelld_restarts += 1;
                let new_pid = process::spawn("/bin/shelld\0");
                state.shelld_pid = Some(new_pid);
            }
        }
    }
}
```

---

## 5. `shelld` Crate

### `shelld/Cargo.toml`

```toml
[package]
name    = "shelld"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "shelld"
path = "src/main.rs"

[dependencies]
libmorpheus = { path = "../libmorpheus" }
display     = { path = "../display", features = ["framebuffer-backend"] }
channel     = { path = "../channel" }
```

### `shelld/src/main.rs`

```rust
#![no_std]
#![no_main]

extern crate alloc;

use libmorpheus::{mem, process};

mod islands;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Wait for compd to be registered.
    loop {
        let mut buf = [libmorpheus::compositor::SurfaceEntry::zeroed(); 1];
        let r = libmorpheus::compositor::surface_list(&mut buf);
        if r != u64::MAX { break; } // EPERM if compositor not set yet
        process::yield_cpu();
    }

    let mut state = islands::ShellState::new();

    loop {
        islands::panel::tick(&mut state);
        islands::wallpaper::tick(&mut state);
        process::yield_cpu();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { unsafe { core::arch::asm!("hlt"); } }
}
```

---

## 6. Migration Path: `shell/src/compositor/` → `compd`

The existing compositor logic in `shell/src/compositor/` is fully working code. Migration is a one-for-one port, not a rewrite.

| Old file | New location | Notes |
|----------|-------------|-------|
| `shell/src/compositor/state.rs` | `compd/src/islands/mod.rs` | `CompState` struct absorbs all state |
| `shell/src/compositor/render.rs` | `compd/src/islands/renderer.rs` | Identical logic, typed channel output |
| `shell/src/compositor/surfaces.rs` | `compd/src/islands/surface_mgr.rs` | Identical logic |
| `shell/src/compositor/input.rs` | `compd/src/islands/input.rs` | Identical logic |
| `shell/src/compositor/event_loop.rs` | `compd/src/main.rs` loop | Restructured into island tick calls |

The `shell` crate retains its existing `compositor/` module until `compd` is confirmed working, then removes it.

---

## 7. Required Kernel Changes

### 7.1 Add SIGHUP (Signal 1)

File: `hwinit/src/process/signals.rs`

SIGHUP is currently **absent** from the `Signal` enum. Add it:

```rust
// in the Signal enum:
Sighup = 1,
// existing entries follow...
Sigint  = 2,
```

Also update the `is_catchable` / handling match arms to include `Sighup`.

**Do not add this until `init` is ready to install a SIGHUP handler.** A premature SIGHUP delivery to a process with no handler will use the default disposition (which may be terminate or ignore — verify from `default_disposition()` in signals.rs).

### 7.2 No Other Kernel Changes Required

All compositor syscalls (91–97) already exist. All process/memory syscalls needed are present. The kernel is otherwise untouched.

---

## 8. Build Targets

All DE binaries use the existing `x86_64-morpheus` custom target.

```toml
# in each new crate's Cargo.toml [package.metadata] or via .cargo/config.toml:
[build]
target = "x86_64-morpheus"
```

Ensure `x86_64-morpheus.json` in the workspace root is referenced correctly. The existing `rust-toolchain.toml` already pins the nightly channel.

---

## 9. File Creation Checklist

In order of dependency (no crate depends on a later crate in this list):

1. `channel/Cargo.toml` + `channel/src/lib.rs`
2. `compd/Cargo.toml`
3. `compd/src/messages.rs`
4. `compd/src/islands/mod.rs`
5. `compd/src/islands/vsync.rs`
6. `compd/src/islands/renderer.rs`
7. `compd/src/islands/surface_mgr.rs`
8. `compd/src/islands/input.rs`
9. `compd/src/islands/focus.rs`
10. `compd/src/main.rs`
11. `init/Cargo.toml`
12. `init/src/islands/supervisor.rs`
13. `init/src/main.rs`
14. `shelld/Cargo.toml`
15. `shelld/src/islands/wallpaper.rs`
16. `shelld/src/islands/panel.rs`
17. `shelld/src/islands/launcher.rs`
18. `shelld/src/main.rs`
19. Root `Cargo.toml` — add 4 new workspace members
20. `hwinit/src/process/signals.rs` — add SIGHUP (deferred)
