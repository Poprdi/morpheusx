# MorpheusX Desktop Environment — Hard Invariants

**Status**: LOCKED  
**These invariants are non-negotiable.** Violating any of them produces memory corruption, kernel panics, security holes, or silent data loss. Each invariant lists its source of truth and the consequence of violation.

---

## Category A — Compositor Ownership

### A1 — Only `compd` may call `compositor_set()`

**Source**: `libmorpheus/src/compositor.rs` → `SYS_COMPOSITOR_SET (91)`  
**Rule**: `compositor_set()` must be called exactly once per `compd` lifetime, as the very first action after `_start`. No other process (including `shelld`, `morphctrl`, or any application) may call it.  
**Exception**: `init` may call `compositor_set()` in the brief window between `compd` dying and a new `compd` being spawned. This reclaims the compositor slot for re-grant.  
**Violation consequence**: The kernel rejects the call with `EPERM` (returns `u64::MAX`). The caller cannot enumerate or map surfaces. Silent failure — the compositor loop will see zero windows forever.

---

### A2 — Only `compd` may call `SYS_FB_MAP`, `SYS_FB_LOCK`, `SYS_FB_UNLOCK`, `SYS_FB_PRESENT`

**Source**: `libmorpheus/src/raw.rs` — syscall numbers 64, 85, 86, 88  
**Rule**: These four syscalls are exclusive to `compd`'s `renderer` island. No other process touches the physical framebuffer.  
**Violation consequence (FB_PRESENT without LOCK)**: Torn frame — the display shows half of `compd`'s frame and half of the interloper's content.  
**Violation consequence (FB_PRESENT from two processes)**: Race on VSync signal. Display hardware may latch corrupted scanlines.

---

### A3 — `compd` owns the `Pipeline` — no other island or process instantiates a second one for the main FB

**Source**: `gfx3d/src/pipeline.rs`, `gfx3d/src/target.rs`  
**Rule**: The `Pipeline` and its associated `SoftwareTarget` in `compd` map directly to the physical framebuffer. A second `Pipeline` instance in another process writing to its own `SurfaceEntry` buffer is permitted (apps may render 3D into their surfaces). A second `Pipeline` writing to the FB pointer is not permitted.  
**Violation consequence**: Two pipelines clearing and writing the same `color_buffer_mut()` simultaneously → completely garbled display output every frame.

---

## Category B — Memory and Pointer Safety

### B1 — Surface pointers are valid only during the `compose()` call

**Source**: `shell/src/compositor/surfaces.rs` (origin), `compd/src/islands/surface_mgr.rs` (destination)  
**Rule**: `CompositeEntry.surface` (`*const u32`) is obtained via `SYS_WIN_SURFACE_MAP (93)`. It is valid as long as the mapping exists. The mapping is torn down in `reap_exited()` via `SYS_MUNMAP`. No island may store this pointer beyond the end of the compositing loop iteration.  
**Violation consequence**: After `reap_exited()` removes a dead process, the pointer points to an unmapped or re-used page. Dereferencing it is a page fault → kernel kills `compd` → `init` must restart the whole compositor stack.

---

### B2 — `SurfaceEntry._pad` and `SurfaceEntry._pad2` must be zero

**Source**: `libmorpheus/src/compositor.rs` — `_pad: u32`, `_pad2: u32` fields  
**Rule**: These fields exist for ABI alignment. The kernel checks that they are zero on certain codepaths. When constructing a `SurfaceEntry` manually (e.g., in tests or morphctrl), zero-initialize the entire struct before setting named fields.  
**Violation consequence**: Kernel may reject the syscall, OR in current implementation may silently misinterpret the struct if padding bytes bleed into adjacent fields on future kernel revisions.

---

### B3 — `FramebufferInfo.stride` is in BYTES, not pixels

**Source**: `display/src/types.rs`, `display/src/framebuffer.rs`  
**Rule**: Pixel address formula: `base + (y * stride) + (x * 4)`. The stride field is byte-width of one scanline, NOT the number of pixels. On a 1920×1080 display with 4 bytes per pixel the stride is 7680, not 1920.  
**Violation consequence**: Every row after the first is read/written from the wrong memory address. Produces a diagonal shear effect on the rendered image. If stride > actual memory allocated, writes go out of bounds → memory corruption.

---

### B4 — `SurfaceEntry.stride` is in PIXELS, not bytes

**Source**: `libmorpheus/src/compositor.rs` — field named `stride: u32`  
**Rule**: This is the OPPOSITE of `FramebufferInfo.stride`. Surface stride = number of `u32` pixels per row in the app's surface buffer. Blit logic must use `src_pitch_bytes = src_stride * 4`.  
**Violation consequence**: Same shear/corruption as B3, but in the source surface read path. Output appears horizontally skewed.

---

### B5 — Channel capacity must be a power of 2

**Source**: `channel/src/lib.rs` — `const ASSERT_POWER_OF_2`  
**Rule**: The ring buffer mask is `N - 1`. This only works as a wrap-around mask when N is a power of 2. The const assertion enforces this at compile time.  
**Violation consequence**: Compile error. This invariant is mechanically enforced and cannot be violated at runtime.

---

### B6 — No allocation inside a signal handler

**Source**: `hwinit/src/process/signals.rs` — signal delivery preempts the normal execution context  
**Rule**: Signal handlers run in the context of the interrupted process, on the same stack, with the heap in an arbitrary intermediate state. Any call to the allocator inside a handler risks deadlock (if the allocator uses a mutex) or corruption (if it does not). Signal handlers in `init` and `compd` must contain only atomic operations, flag writes, and syscalls.  
**Violation consequence**: Heap corruption or deadlock. On a no_std system with a buddy allocator, this typically manifests as a panic on the next allocation, far from the actual cause.

---

## Category C — Island Isolation

### C1 — No island may hold a reference to another island's exclusive state

**Source**: Island architecture pattern (C)  
**Rule**: Each island owns its state exclusively. State is passed by value in messages, not by reference. No island holds a `&T` or `&mut T` pointing into another island's data, and no `Arc<Mutex<T>>` or equivalent may cross island boundaries.  
**Violation consequence**: The compile-time single-ownership guarantee breaks down. The motivation for island isolation (predictable crash boundaries, no shared mutable state bugs) is completely negated. Every mutation becomes a potential race if the scheduler model ever changes.

---

### C2 — The `renderer` island is the sole caller of pixel-write primitives on the FB

**Source**: `display/src/framebuffer.rs` — `put_pixel`, `fill_rect` (both ASM-backed)  
**Rule**: Only `renderer.rs` calls `Framebuffer::put_pixel`, `Framebuffer::fill_rect`, `asm_fb_write32`, `asm_fb_memset`, `asm_fb_memcpy` on the physical framebuffer handle. Other islands may hold a READ reference to `fb_info` (for width/height queries) but must not write.  
**Violation consequence**: Two islands simultaneously writing different regions of the FB. On a single-core system this is currently benign (no actual parallelism) but the dependency on single-core is invisible to the compiler, so any future threading change immediately produces corruption.

---

### C3 — The `surface_mgr` island is the sole caller of surface syscalls

**Source**: `libmorpheus/src/compositor.rs`  
**Rule**: Only `surface_mgr` calls `SYS_WIN_SURFACE_LIST (92)`, `SYS_WIN_SURFACE_MAP (93)`, `SYS_WIN_SURFACE_DIRTY_CLEAR (95)`. The `input` island may read the window array (by value copy / snapshot) to perform hit-testing but must not call these syscalls.  
**Violation consequence**: Double-map: same physical pages mapped twice into `compd`'s address space at different virtual addresses. Both pointers appear valid. Writing through either one is consistent, but `SYS_MUNMAP` on only one pointer leaves a dangling mapping. The kernel may reclaim the physical pages while `surface_ptr` is still in use.

---

### C4 — The `input` island is the sole caller of forward syscalls

**Source**: `libmorpheus/src/compositor.rs`, `libmorpheus/src/raw.rs`  
**Rule**: Only `input` calls `SYS_FORWARD_INPUT (97)` and `SYS_MOUSE_FORWARD (94)`. No other island sends input events to application processes.  
**Violation consequence**: Application receives duplicate or out-of-order input events. For a text editor, this means characters are inserted twice. For a game, mouse sensitivity doubles.

---

## Category D — Process Lifecycle

### D1 — `init` must reclaim the compositor slot before re-spawning `compd`

**Source**: `SYS_COMPOSITOR_SET (91)` — kernel enforces one holder  
**Rule**: When `compd` dies, the kernel may or may not automatically release the compositor slot (behavior depends on kernel implementation). `init` must call `compositor_set()` itself before spawning a new `compd`, to ensure the slot is available.  
**Violation consequence**: New `compd` fails on its first `compositor_set()` call → receives `EPERM` → no surfaces can be enumerated → blank screen. The system appears dead. `init`'s restart counter increments and eventually stops trying.

---

### D2 — `init` must not restart a component more than `MAX_RESTARTS` times

**Source**: `init/src/islands/supervisor.rs` — `MAX_RESTARTS = 5` (defined in implementation)  
**Rule**: If `compd` or `shelld` crashes more than `MAX_RESTARTS` times, `init` stops restarting it and enters a degraded state. It must NOT loop forever spawning a crashing process.  
**Violation consequence**: PID table fills up (`MAX_PROCESSES = 64` from `hwinit/src/process/mod.rs`). Once full, `SYS_SPAWN` returns error. All subsequent process creation in the entire system fails, including launching user applications.

---

### D3 — `compd` must call `SYS_COMPOSITOR_SET` before `SYS_WIN_SURFACE_LIST`

**Source**: `libmorpheus/src/compositor.rs` — `compositor_set()` then `surface_list()`  
**Rule**: `SYS_WIN_SURFACE_LIST (92)` returns `EPERM` (u64::MAX) if the calling process has not registered as compositor via `SYS_COMPOSITOR_SET (91)`.  
**Violation consequence**: `surface_list()` always returns `u64::MAX`. The count is interpreted as `usize::MAX` entries. Iterating over that count reads undefined memory. Immediate panic or memory corruption.

---

### D4 — `shelld` must wait for `compd` to be registered before calling surface syscalls

**Source**: `shelld/src/main.rs` — startup wait loop  
**Rule**: `shelld` polls `SYS_WIN_SURFACE_LIST` at startup. If it receives `EPERM`, it `SYS_YIELD`s and retries. It must not proceed to render or register its surface until `compd` is active.  
**Violation consequence**: `shelld`'s surface is registered with the kernel, but `compd` is not yet the compositor. When `compd` starts and calls `SYS_WIN_SURFACE_LIST`, it won't see `shelld`'s surface if it was registered before the compositor slot was taken. (Behavior depends on kernel implementation; assume worst case: surface is invisible.)

---

## Category E — Pixel Format

### E1 — `pack(r, g, b)` must branch on `is_bgrx`

**Source**: `display/src/types.rs` — `PixelFormat::Bgrx = 1` (UEFI default), `PixelFormat::Rgbx = 0`  
**Rule**: `compd` reads `FramebufferInfo.format` at startup to determine if the display is BGRX or RGBX. The `pack()` function in the renderer MUST branch on this. Hard-coding BGRX for all platforms will display red and blue channels swapped on RGBX hardware (rare but valid UEFI implementations).  
**Violation consequence**: Red channels appear on blue positions and vice versa. The desktop background `(26, 26, 46)` would appear slightly different but still dark. App content would have swapped red/blue in all colors. No crash — purely visual corruption that is insidious to debug.

---

### E2 — App surface buffers must use the same pixel format as the framebuffer

**Source**: `compd/src/islands/renderer.rs` — `blit_surface()` copies pixels verbatim  
**Rule**: The blit operation in `renderer.rs` copies `u32` pixels from the surface buffer directly to the framebuffer without format conversion. Apps must write their pixels in the same format that `compd`'s framebuffer uses (BGRX on standard UEFI hardware).  
**Violation consequence**: App content is displayed with swapped color channels. No crash. Silent visual corruption.

---

## Category F — Arithmetic and Overflow

### F1 — `SoftwareTarget` depth buffer uses reversed-Z

**Source**: `gfx3d/src/target.rs` — `SoftwareTarget::new()` initializes depth to `0xFFFF_FFFF`  
**Rule**: Depth values are 16.16 unsigned fixed-point. `0xFFFF_FFFF` means "infinitely far". Closer objects have LOWER depth values. A fragment passes the depth test if `fragment_depth < depth_buffer[pixel]`. Do NOT negate depth values or use `>`/`>=` in the depth comparison.  
**Violation consequence**: All geometry fails the depth test → nothing is drawn. Or: all geometry passes the depth test → back faces overwrite front faces. Either way the rendered output is completely wrong.

---

### F2 — Mouse coordinates must be clamped to `[0, fb_w-1]` × `[0, fb_h-1]`

**Source**: `shell/src/compositor/input.rs` (origin) → `compd/src/islands/input.rs`  
**Rule**: Mouse delta (`dx`, `dy`) from `SYS_MOUSE_READ (84)` is a signed delta in device units. `mouse_x` and `mouse_y` must be clamped after each update. The cursor drawing code writes a 2×2 pixel block at `(mouse_x, mouse_y)` — an out-of-bounds coordinate writes outside the allocated framebuffer region.  
**Violation consequence**: Out-of-bounds write to framebuffer-adjacent memory. Corrupts whatever follows the framebuffer in physical memory. Typically manifests as random corruption of kernel or other process data.

---

### F3 — Channel send returning `Err(msg)` must not be silently discarded

**Source**: `channel/src/lib.rs` — `send()` returns `Result<(), T>`  
**Rule**: If the channel is full, `send()` returns `Err(msg)`. The message is lost. The caller must handle this. Acceptable handling: drop with a debug log, OR retry after draining on the consumer side. Silent discard (`.ok()` with no logging) makes the system appear to work while losing events.  
**Violation consequence**: Focus change events are lost → keyboard focus is stuck. Input events are lost → the user cannot type in an application. These failures are non-deterministic and extremely difficult to reproduce.

---

## Summary Table

| ID | Rule | Consequence |
|----|------|-------------|
| A1 | Only compd calls compositor_set() | EPERM, zero windows, blank screen |
| A2 | Only compd calls FB syscalls | Torn/garbled display |
| A3 | Only compd's renderer writes to the main FB Pipeline | Garbled display every frame |
| B1 | Surface pointers valid only during compose() | Page fault, compd dies, init restarts |
| B2 | SurfaceEntry pads must be zero | ABI mismatch, kernel rejects |
| B3 | FramebufferInfo.stride is in bytes | Memory corruption / display shear |
| B4 | SurfaceEntry.stride is in pixels | Display shear in blitted surfaces |
| B5 | Channel N must be power of 2 | Compile error (mechanically enforced) |
| B6 | No alloc in signal handlers | Heap corruption or deadlock |
| C1 | No cross-island references | Loss of isolation guarantees |
| C2 | Only renderer calls FB write primitives | Potential corruption on threading |
| C3 | Only surface_mgr calls surface syscalls | Double-map, dangling pointer |
| C4 | Only input calls forward syscalls | Duplicate input events in apps |
| D1 | init reclaims compositor slot before restart | EPERM, new compd can't start |
| D2 | Max restart limit respected | PID table exhaustion |
| D3 | compositor_set() before surface_list() | u64::MAX count → memory corruption |
| D4 | shelld waits for compd before surface ops | Invisible shelld surface |
| E1 | pack() branches on is_bgrx | Swapped R/B channels on display |
| E2 | App surfaces use same pixel format as FB | Swapped R/B in app content |
| F1 | Depth buffer is reversed-Z | All geometry invisible or inverted |
| F2 | Mouse coordinates clamped to FB bounds | Out-of-bounds memory write |
| F3 | Channel send Err must not be silently dropped | Lost events, stuck UI state |
