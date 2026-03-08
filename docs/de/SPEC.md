# MorpheusX Desktop Environment — Architecture Specification

**Status**: LOCKED  
**Architecture Pattern**: Island (C) — Deterministic Islands, Typed Message Channels  
**Verified against source**: `libmorpheus/src/raw.rs`, `shell/src/compositor/`, `hwinit/src/process/`, `display/src/`, `gfx3d/src/`

---

## 1. Executive Summary

The MorpheusX DE is a four-process userspace system built on the Island architecture pattern. There is no shared mutable state between islands. All cross-island communication flows through fixed-capacity ring buffer channels carrying typed message enums. Each process crashes independently; the supervisor (`init`) restarts failed components without kernel involvement.

The compositor (`compd`) is the **sole owner** of the display hardware and 3D rendering pipeline. Applications render to shared-memory surface buffers; `compd` blends them onto the physical framebuffer on each vsync tick. This design preserves the existing Quake-class software renderer asset without modification.

---

## 2. System Topology

```
┌──────────────────────────────────────────────────────────┐
│  init  (PID 1)                                           │
│  ┌────────────────┐  ┌──────────────────────────────┐    │
│  │  supervisor    │  │  ipc routing                 │    │
│  │  island        │  │  island                      │    │
│  └────────────────┘  └──────────────────────────────┘    │
└──────────────────────────────────────────────────────────┘
         │ SYS_SPAWN                    │ SYS_SPAWN
         ▼                              ▼
┌──────────────────────┐     ┌────────────────────────────┐
│  compd               │     │  shelld                    │
│  ┌────────────┐       │     │  ┌──────────┐              │
│  │ vsync      │       │     │  │ wallpaper│              │
│  │ renderer   │       │     │  │ panel    │              │
│  │ surface_mgr│       │     │  │ launcher │              │
│  │ input      │       │     │  └──────────┘              │
│  │ focus      │       │     └────────────────────────────┘
│  └────────────┘       │
└──────────────────────┘
         ▲
         │ morphctrl (CLI tool, short-lived processes)
         │ SYS_FORWARD_INPUT / SYS_KILL
```

### Process Summary

| Process | Role | Crates Used | Persistent |
|---------|------|-------------|------------|
| `init` | PID 1 supervisor, restarts dead processes | `libmorpheus` | Yes |
| `compd` | Compositor, owns FB + gfx3d | `libmorpheus`, `display`, `gfx3d` | Yes |
| `shelld` | Desktop shell (wallpaper, panel, launcher) | `libmorpheus`, `display` | Yes |
| `morphctrl` | CLI control utility | `libmorpheus` | No (short-lived) |

---

## 3. Island Definitions

An **island** is a named logical unit of isolated state and behavior running within a process. Islands within the same process communicate via typed channel messages. Islands across process boundaries communicate via kernel syscalls only.

### 3.1 `compd` Islands

#### Island: `vsync`
- **Owns**: the vsync tick, `SYS_CLOCK` polling cadence
- **State**: `last_tick_ns: u64`, `target_frame_ns: u64` (16.6ms for 60 Hz)
- **Produces**: `VsyncMsg::Tick { now_ns: u64 }` onto the vsync channel
- **Consumes**: nothing from other islands
- **Invariant**: only this island calls `SYS_CLOCK`

#### Island: `renderer`
- **Owns**: `Pipeline` (from `gfx3d`), `SoftwareTarget`, the framebuffer pointer
- **State**: `pipeline: Pipeline`, `target: SoftwareTarget`, `fb: Framebuffer`, `fb_info: FramebufferInfo`
- **Produces**: `RendererMsg::FrameDone { stats: FrameStats }` after each `fb_present()`
- **Consumes**: `VsyncMsg::Tick` (start frame), `SurfaceMsg::CompositeList { entries: [CompositeEntry; MAX_WINDOWS] }` (what to draw)
- **Invariant**: sole caller of `SYS_FB_LOCK`, `SYS_FB_UNLOCK`, `SYS_FB_PRESENT`, `SYS_FB_MAP`. No other island or process may hold the FB pointer.

#### Island: `surface_mgr`
- **Owns**: the window array (`[Option<ChildWindow>; MAX_WINDOWS]`), cascade state
- **State**: `windows: [Option<ChildWindow>; MAX_WINDOWS]`, `cascade_n: u32`
- **Produces**: `SurfaceMsg::CompositeList` after every `SYS_WIN_SURFACE_LIST` poll
- **Consumes**: `InputMsg::WindowMoved`, `InputMsg::WindowResized`, `InputMsg::WindowClosed`, `FocusMsg::FocusChanged`
- **Invariant**: sole caller of `SYS_WIN_SURFACE_LIST (92)`, `SYS_WIN_SURFACE_MAP (93)`, `SYS_WIN_SURFACE_DIRTY_CLEAR (95)`, `SYS_MUNMAP` on surface pages

#### Island: `input`
- **Owns**: raw keyboard/mouse state, hit-testing logic
- **State**: `mouse_x: i32`, `mouse_y: i32`, `last_buttons: u32`, `capture: MouseCapture`
- **Produces**: `InputMsg::*` (WindowMoved, WindowResized, WindowClosed, KeyForward, MouseForward)
- **Consumes**: stdin byte stream (PS/2 keyboard scan codes), `SYS_MOUSE_READ (84)`
- **Invariant**: sole caller of `SYS_FORWARD_INPUT (97)`, `SYS_MOUSE_FORWARD (94)`. Hit-test uses window rects from `surface_mgr` via last `CompositeList` snapshot (read-only copy, no lock).

#### Island: `focus`
- **Owns**: focused window index, z-order policy
- **State**: `focused: Option<u8>`, `window_count: u8`
- **Produces**: `FocusMsg::FocusChanged { old: Option<u8>, new: Option<u8> }`
- **Consumes**: `InputMsg::WindowClosed`, `InputMsg::KeyForward` (Ctrl+] cycle)
- **Invariant**: focus changes are only performed by this island. No other island mutates the focused index.

### 3.2 `shelld` Islands

#### Island: `wallpaper`
- **Owns**: wallpaper `SurfaceEntry` rendered into its own surface buffer
- **State**: `surface_buf: Vec<u32>`, `width: u32`, `height: u32`
- **Produces**: nothing (writes surface, sets dirty via libmorpheus surface protocol)
- **Consumes**: `ShellMsg::Repaint`
- **Invariant**: shelld's surface is registered with the kernel via normal surface protocol; `compd` blends it at z-layer 0 (background).

#### Island: `panel`
- **Owns**: taskbar rendering state (clock, window list, tray)
- **State**: `window_list: [Option<PanelEntry>; MAX_WINDOWS]`, `clock_str: [u8; 16]`
- **Produces**: dirty surface writes
- **Consumes**: `ShellMsg::WindowListUpdate`, `VsyncMsg::Tick` (clock refresh)

#### Island: `launcher`
- **Owns**: application launcher overlay state
- **State**: `visible: bool`, `selected: u8`, `app_list: &'static [AppEntry]`
- **Produces**: `ShellMsg::LaunchApp { path: [u8; 256] }`
- **Consumes**: `InputMsg::KeyForward` (Super key), `ShellMsg::LaunchApp` ack

### 3.3 `init` Islands

#### Island: `supervisor`
- **Owns**: process birth/death state for `compd` and `shelld`
- **State**: `compd_pid: Option<u32>`, `shelld_pid: Option<u32>`
- **Produces**: `SupervisorMsg::Restarting { name: &'static str }` (logged to serial)
- **Consumes**: `SIGCHLD` delivery (via `SYS_SIGACTION`), then `SYS_TRY_WAIT (96)`
- **Invariant**: on `compd` death, `init` calls `SYS_COMPOSITOR_SET` with its own PID to reclaim the compositor slot before re-spawning `compd`.

#### Island: `ipc` (future)
- Placeholder for a named socket routing system. Not implemented in v1.

---

## 4. Channel Protocol

### 4.1 Ring Buffer Layout

Each channel is a fixed-capacity SPSC (single-producer, single-consumer) ring buffer. Within a single process, cross-island channels are in the **same address space** — no syscalls needed for message passing. The ring buffer is not used across process boundaries (syscalls handle that).

```
struct Channel<T, const N: usize> {
    buf:    [MaybeUninit<T>; N],  // N must be a power of 2
    head:   AtomicUsize,          // written by producer
    tail:   AtomicUsize,          // written by consumer
}
```

- `N` must be a power of 2. The mask is `N - 1`.
- `send()`: read head, check `(head - tail) < N`, write `buf[head & mask]`, increment head. Returns `Err(Full)` if full — **no blocking, no allocation**.
- `recv()`: read tail, check `tail != head`, read `buf[tail & mask]`, increment tail. Returns `None` if empty.
- `MaybeUninit<T>` avoids running destructors on unread slots.

### 4.2 Message Types

#### `VsyncMsg`
```rust
pub enum VsyncMsg {
    Tick { now_ns: u64 },
}
```

#### `SurfaceMsg`
```rust
pub struct CompositeEntry {
    pub pid:        u32,
    pub surface:    *const u32,  // mapped virtual address
    pub x: i32, pub y: i32,
    pub w: u32, pub h: u32,
    pub src_stride: u32,         // in pixels
    pub z_layer:    u8,          // 0=bg,1=bottom,2=top,3=overlay
    pub dirty:      bool,
}

pub enum SurfaceMsg {
    CompositeList {
        entries: [Option<CompositeEntry>; MAX_WINDOWS],
        count:   u8,
    },
}
```

#### `InputMsg`
```rust
pub enum InputMsg {
    WindowMoved   { idx: u8, new_x: i32, new_y: i32 },
    WindowResized { idx: u8, new_w: u32, new_h: u32 },
    WindowClosed  { idx: u8, pid: u32 },
    KeyForward    { pid: u32, scancode: u8 },
    MouseForward  { pid: u32, dx: i16, dy: i16, buttons: u8 },
}
```

#### `FocusMsg`
```rust
pub enum FocusMsg {
    FocusChanged { old: Option<u8>, new: Option<u8> },
}
```

#### `ShellMsg`
```rust
pub enum ShellMsg {
    Repaint,
    WindowListUpdate { count: u8, titles: [[u8; 64]; MAX_WINDOWS] },
    LaunchApp        { path: [u8; 256] },
}
```

#### `SupervisorMsg`
```rust
pub enum SupervisorMsg {
    Restarting { name: [u8; 16] },
    Started    { name: [u8; 16], pid: u32 },
    GaveUp     { name: [u8; 16] },
}
```

---

## 5. Z-Layer Model

`compd` assigns each surface a z-layer at composite time. Layers are blended in ascending order; higher layers occlude lower layers.

| Layer | Value | Assigned To |
|-------|-------|-------------|
| background | 0 | `shelld` wallpaper surface |
| bottom | 1 | Normal application windows |
| top | 2 | Floating/always-on-top windows (future) |
| overlay | 3 | `shelld` panel surface, lock screen |

Z-ordering within a layer: unfocused windows first, focused window last (painter's algorithm). This matches the existing `compose()` logic in `shell/src/compositor/render.rs`.

---

## 6. Surface Lifecycle

```
App process                    Kernel                    compd
     │                           │                         │
     │  SYS_MMAP (anon)          │                         │
     │──────────────────────────►│                         │
     │  returns virtual addr     │                         │
     │◄──────────────────────────│                         │
     │                           │                         │
     │  write pixels to buffer   │                         │
     │  set dirty flag           │                         │
     │                           │                         │
     │                           │   SYS_WIN_SURFACE_LIST  │
     │                           │◄────────────────────────│
     │                           │   [SurfaceEntry×N]      │
     │                           │────────────────────────►│
     │                           │                         │
     │                           │   SYS_WIN_SURFACE_MAP   │
     │                           │◄────────────────────────│
     │                           │   *mut u8 (mapped)      │
     │                           │────────────────────────►│
     │                           │                         │
     │                           │   (compd composites)    │
     │                           │                         │
     │                           │  SYS_WIN_SURFACE_DIRTY  │
     │                           │◄──────── _CLEAR ────────│
```

**Dirty protocol**: The `dirty` field in `SurfaceEntry` is a `u32` written by the app and cleared by `compd` via `SYS_WIN_SURFACE_DIRTY_CLEAR (95)`. `compd` does NOT skip surfaces where `dirty == 0` — it still composites them (avoids ghosting). The dirty flag is purely advisory for future optimization.

---

## 7. Compositor Ownership Model

### 7.1 `compositor_set()` Contract
- Kernel enforces one registered compositor at a time.
- `SYS_COMPOSITOR_SET (91)`: callable only once per boot unless the current holder dies or explicitly releases.
- `compd` calls this exactly once at startup, before calling `SYS_WIN_SURFACE_LIST`.
- `init` may temporarily reclaim it between `compd` crashes and restarts.

### 7.2 Framebuffer Ownership
- `compd` calls `SYS_FB_MAP (64)` once at startup to get the framebuffer virtual address.
- `compd` holds this pointer for its entire lifetime.
- No other process calls `SYS_FB_MAP`, `SYS_FB_LOCK (85)`, `SYS_FB_UNLOCK (86)`, `SYS_FB_PRESENT (88)`.

### 7.3 3D Pipeline Ownership
- `compd`'s `renderer` island owns the single `Pipeline` instance.
- `shelld` does NOT use `gfx3d`. It renders 2D only (direct pixel writes to its surface buffer).
- Application processes may create their own `Pipeline` if they link `gfx3d`, but they do NOT get framebuffer access — they render into their surface buffer.

---

## 8. Existing Constants (Verified from Source)

All values below are confirmed from `shell/src/compositor/state.rs` and `libmorpheus/src/raw.rs`.

### Window Manager Constants
```
MAX_WINDOWS  = 16
TITLE_H      = 22   // pixels
BORDER       = 1    // pixels
CASCADE_STEP = 28   // pixels
```

### Colors (RGB tuples, rendered as BGRX on default hardware)
```
DESKTOP_RGB        = (26, 26, 46)    // dark blue-black background
TITLE_FOCUSED_RGB  = (0, 85, 0)     // dark green title bar when focused
TITLE_UNFOCUSED    = dimmed variant
BORDER_FOCUSED_RGB = (0, 170, 0)    // bright green border when focused
CURSOR_RGB         = (255, 255, 255) // white mouse cursor
```

### Syscall Numbers (confirmed from `libmorpheus/src/raw.rs`)
```
SYS_YIELD                   =  3
SYS_GETPID                  =  6
SYS_KILL                    =  7
SYS_WAIT                    =  8
SYS_SLEEP                   =  9
SYS_CLOCK                   = 22
SYS_SPAWN                   = 25
SYS_MMAP                    = 26
SYS_MUNMAP                  = 27
SYS_FB_INFO                 = 63
SYS_FB_MAP                  = 64
SYS_SIGACTION               = 66
SYS_SHM_GRANT               = 73
SYS_FUTEX                   = 79
SYS_MOUSE_READ              = 84
SYS_FB_LOCK                 = 85
SYS_FB_UNLOCK               = 86
SYS_FB_IS_LOCKED            = 87
SYS_FB_PRESENT              = 88
SYS_FB_BLIT                 = 89
SYS_FB_MARK_DIRTY           = 90
SYS_COMPOSITOR_SET          = 91
SYS_WIN_SURFACE_LIST        = 92
SYS_WIN_SURFACE_MAP         = 93
SYS_MOUSE_FORWARD           = 94
SYS_WIN_SURFACE_DIRTY_CLEAR = 95
SYS_TRY_WAIT                = 96
SYS_FORWARD_INPUT           = 97
```

---

## 9. Signal Model

Confirmed from `hwinit/src/process/signals.rs`:

| Signal | Number | Catchable | Usage in DE |
|--------|--------|-----------|-------------|
| SIGINT  | 2 | Yes | Ctrl+C forwarded by `compd` to focused app |
| SIGKILL | 9 | **No** | `morphctrl kill <pid>` |
| SIGSEGV | 11 | Yes | App crash — `compd` reaps on `SYS_TRY_WAIT` |
| SIGTERM | 15 | Yes | Graceful shutdown request |
| SIGCHLD | 17 | Yes | `init` watches for `compd`/`shelld` death |
| SIGCONT | 18 | Yes | Future: resume suspended app |
| SIGSTOP | 19 | **No** | Future: suspend app |

> **SIGHUP (1) is NOT implemented.** It is absent from the `Signal` enum in `hwinit/src/process/signals.rs`. Do not rely on it for config reload. Use a dedicated `SYS_FORWARD_INPUT` message sequence or a named pipe instead.

---

## 10. Process Lifecycle

### `init` startup sequence
1. `init` is PID 1 (spawned by bootloader/UEFI stage).
2. `init` spawns `compd` via `SYS_SPAWN (25)`.
3. `init` spawns `shelld` via `SYS_SPAWN (25)`.
4. `init` installs `SIGCHLD` handler via `SYS_SIGACTION (66)`.
5. `init` enters its supervisor loop: `SYS_TRY_WAIT (96)` → restart logic → `SYS_YIELD (3)`.

### `compd` startup sequence
1. Call `SYS_GETPID (6)` — record own PID.
2. Call `compositor_set()` → `SYS_COMPOSITOR_SET (91)`.
3. Call `SYS_FB_MAP (64)` → get framebuffer pointer + `FramebufferInfo`.
4. Detect pixel format (`PixelFormat::Bgrx` = 1 is the UEFI default).
5. Initialize `renderer` island: construct `Pipeline`, `SoftwareTarget`, `Framebuffer`.
6. Enter main vsync loop.

### `compd` main loop (per vsync tick)
```
poll_stdin()           // feed raw bytes to input island
poll_mouse()           // SYS_MOUSE_READ (84)
handle_input()         // hit-test, update capture, produce InputMsg
update_surfaces()      // SYS_WIN_SURFACE_LIST (92), SYS_WIN_SURFACE_MAP (93)
compose()              // clear FB, z-order draw, draw_cursor
fb_present()           // SYS_FB_PRESENT (88)
dirty_clear_all()      // SYS_WIN_SURFACE_DIRTY_CLEAR (95) for each window
reap_exited()          // SYS_TRY_WAIT (96) for each known pid
yield_cpu()            // SYS_YIELD (3)
```

### `shelld` startup sequence
1. Wait for `compd` to be registered (poll `SYS_WIN_SURFACE_LIST` until not `EPERM`).
2. `SYS_MMAP` a surface buffer for wallpaper.
3. Render wallpaper into buffer.
4. Set dirty.
5. Enter panel/launcher loop.

---

## 11. `morphctrl` Interface

`morphctrl` is a short-lived CLI process. It communicates with `compd` and the kernel directly via syscalls. It does NOT connect to `init` via IPC in v1.

### Planned subcommands
```
morphctrl list                  # SYS_WIN_SURFACE_LIST, print pid/title/geometry
morphctrl focus <pid>           # SYS_FORWARD_INPUT to compd with focus-change event
morphctrl kill <pid>            # SYS_KILL (7) with SIGKILL (9)
morphctrl screenshot <path>     # read FB via SYS_FB_MAP, write PNG/PPM to disk
```

---

## 12. Framebuffer and Pixel Format

### `FramebufferInfo` layout (confirmed from `display/src/types.rs`)
```rust
pub struct FramebufferInfo {
    pub base:   u64,     // physical base address
    pub size:   usize,   // total byte size
    pub width:  u32,     // pixels
    pub height: u32,     // pixels
    pub stride: u32,     // **BYTES** per row (NOT pixels)
    pub format: PixelFormat,
}
```

> **Stride is in BYTES.** Pixel address formula: `base + (y * stride) + (x * 4)`. All writes must use this formula or the ASM primitives from `display/src/framebuffer.rs`.

### `PixelFormat`
```rust
pub enum PixelFormat {
    Rgbx   = 0,
    Bgrx   = 1,  // default on UEFI GOP hardware
    BitMask = 2,
    BltOnly = 3,
}
```

`pack(r, g, b)` in `compd` must branch on format:
- `Bgrx`: `(b as u32) | ((g as u32) << 8) | ((r as u32) << 16)`
- `Rgbx`: `(r as u32) | ((g as u32) << 8) | ((b as u32) << 16)`

---

## 13. `SurfaceEntry` ABI (kernel-defined, do not change)

Confirmed from `libmorpheus/src/compositor.rs`:

```rust
#[repr(C)]
pub struct SurfaceEntry {
    pub pid:       u32,
    pub _pad:      u32,    // MUST be zero
    pub phys_addr: u64,
    pub pages:     u64,
    pub width:     u32,
    pub height:    u32,
    pub stride:    u32,    // in pixels (NOT bytes) for surface buffers
    pub format:    u32,
    pub dirty:     u32,
    pub _pad2:     u32,    // MUST be zero
}
```

> `SurfaceEntry.stride` is in **pixels** (not bytes). This differs from `FramebufferInfo.stride` which is in bytes. Do not conflate them.

---

## 14. Future Considerations (Out of Scope for v1)

- **GPU backend**: swap `display`'s `FramebufferBackend` for a GPU DMA backend inside `compd` only. No app changes needed.
- **Wayland-equivalent protocol**: replace `SYS_WIN_SURFACE_*` with a richer protocol without breaking the island boundaries.
- **SIGHUP reload**: requires adding `SIGHUP = 1` to `hwinit/src/process/signals.rs` first.
- **Multi-monitor**: `FramebufferInfo` is per-FB; `compd` would need multiple `renderer` islands with separate `Pipeline` instances.
- **Named pipe IPC**: `SYS_PIPE (75)` + `SYS_DUP2 (76)` already exist; can replace the channel-over-mmap approach.
