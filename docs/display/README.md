# MorpheusX Display Stack (virtio-gpu)

GPU display and rendering stack for post-ExitBootServices bare-metal environment.

### Design Principles

1. **Platform Abstraction**: `DisplayDriver` trait allows multiple implementations (virtio-gpu baseline, vendor GPUs, simple framebuffer fallback)
2. **No External Deps**: Everything built on primitives
3. **Error Handling**: Comprehensive error types for display operations
4. **Hardware Acceleration Priority**: Prefer 3D-capable transports (virtio-gpu with virgl/3D, vendor GPUs); fallback to 2D blit with explicit degraded-mode labeling
5. **Deterministic Control**: Explicit state machines, no hidden state

## Performance Targets

| Target | Value | Notes |
|--------|-------|-------|
| Frame Rate | ≥60 FPS | Hardware acceleration required |
| Frame Budget | ≤16.6 ms | Per-frame time limit |
| Jitter | ≤2 ms | Frame pacing variance target |
| Buffering | Double/Triple | Configurable vsync strategy |

## Usage

### Basic Frame Upload

```rust
use morpheus_display::{DisplayDriver, VirtioGpuDriver, FrameBuffer};

// Create virtio-gpu driver from handoff
let mut driver = VirtioGpuDriver::new(handoff)?;

// Initialize display mode
driver.set_mode(1920, 1080, PixelFormat::BGRA8888)?;

// Allocate scanout resource
let scanout = driver.create_scanout(0, 1920, 1080)?;

// Frame rendering loop
loop {
    // Get frame buffer for rendering
    let fb = driver.acquire_frame_buffer()?;
    
    // Render content to frame buffer
    render_to_buffer(&mut fb);
    
    // Upload and present
    driver.upload_resource(fb)?;
    driver.set_scanout(0, scanout)?;
    driver.flush()?;
}
```

### 3D Rendering Path (virgl/3D Feature)

```rust
use morpheus_display::{Display3D, CommandBuffer, Fence};

// Check for 3D capability
if driver.capabilities().has_3d {
    // Create 3D context
    let ctx = driver.create_3d_context()?;
    
    // Build command buffer
    let mut cmds = CommandBuffer::new();
    cmds.set_viewport(0, 0, 1920, 1080);
    cmds.clear_color(0.0, 0.0, 0.0, 1.0);
    cmds.draw_triangles(&vertices, &indices);
    
    // Submit with fence for synchronization
    let fence = driver.submit_3d(ctx, cmds)?;
    
    // Non-blocking fence check (poll in main loop)
    if driver.fence_signaled(fence)? {
        // Frame complete, swap buffers
        driver.present()?;
    }
}
```

### Signaling and Fencing

```rust
// Explicit CPU/GPU synchronization
let fence_id = driver.create_fence()?;
driver.attach_fence(fence_id, resource)?;
driver.flush()?;

// Poll for completion (non-blocking)
loop {
    if driver.poll_fence(fence_id)? {
        break;
    }
    // Yield to other main loop phases
}
```

## Implementation

### See /docs/md/Architecture&Design/Display/

Detailed documentation:
- `IMPLEMENTATION_GUIDE.md` — Implementation reference
- `DISPLAY_ASM_RUST_ABI_CONTRACT.md` — Frozen ABI specification
- `UEFI_COMPAT_LAYER.md` — UEFI compatibility shim documentation
- `HARDWARE_EXTENSION_GUIDE.md` — Drop-in driver architecture

## API Overview

### High-Level Display API

| Function | Description |
|----------|-------------|
| `init(handoff)` | Initialize display from boot handoff |
| `set_mode(w, h, fmt)` | Configure display resolution and format |
| `create_resource(w, h, fmt)` | Create GPU-side resource |
| `upload(resource, data)` | Upload pixel data to resource |
| `set_scanout(id, resource)` | Attach resource to scanout |
| `flush()` | Flush pending operations |
| `create_fence()` | Create synchronization fence |
| `poll_fence(id)` | Non-blocking fence status check |

### Driver Abstraction Layer

```rust
pub trait DisplayDriver {
    fn capabilities(&self) -> &DisplayCapabilities;
    fn set_mode(&mut self, width: u32, height: u32, format: PixelFormat) -> Result<(), DisplayError>;
    fn create_resource(&mut self, width: u32, height: u32, format: PixelFormat) -> Result<ResourceId, DisplayError>;
    fn upload_resource(&mut self, id: ResourceId, data: &[u8]) -> Result<(), DisplayError>;
    fn set_scanout(&mut self, scanout_id: u32, resource: ResourceId) -> Result<(), DisplayError>;
    fn flush(&mut self) -> Result<(), DisplayError>;
    fn create_fence(&mut self) -> Result<FenceId, DisplayError>;
    fn poll_fence(&mut self, id: FenceId) -> Result<bool, DisplayError>;
    fn teardown(&mut self);
}
```

## Error Handling

All operations return `Result<T, DisplayError>`:

```rust
match driver.set_mode(1920, 1080, PixelFormat::BGRA8888) {
    Ok(()) => { /* success */ },
    Err(DisplayError::ModeNotSupported) => {
        // Try fallback resolution
    },
    Err(DisplayError::ResourceExhausted) => {
        // Out of GPU memory
    },
    Err(DisplayError::DeviceError(code)) => {
        // Hardware error
    },
    Err(e) => {
        // Other error
    }
}
```

## UEFI Compatibility

For backward compatibility with existing UEFI display code:

```rust
#[cfg(feature = "uefi-compat")]
use morpheus_display::compat::UefiDisplayShim;

// Create shim that exposes legacy UEFI GOP-like API
let shim = UefiDisplayShim::new(&mut driver);

// Legacy API calls work transparently
shim.blt(buffer, BltOperation::BufferToVideo, 0, 0, 0, 0, 1920, 1080)?;
```

Compile with `--features uefi-compat` to enable the compatibility layer.

## Testing

Test in QEMU with virtio-gpu:
```bash
cd testing
./run.sh
```

Ensure QEMU has virtio-gpu configured:
```bash
-device virtio-gpu-pci,virgl=on \
-display gtk,gl=on
```

For 3D testing (requires virglrenderer):
```bash
-device virtio-gpu-pci,virgl=on,max_outputs=1 \
-display sdl,gl=on
```

## Future: Hardware Extension

When adding real GPU support (AMD, Intel, NVIDIA):
- Implement `DisplayDriver` trait for vendor GPU
- Drop-in ASM driver implementing common capability interface
- Same high-level API works unchanged
- See `HARDWARE_EXTENSION_GUIDE.md` for details

## References

- VirtIO GPU Specification v1.2
- UEFI Graphics Output Protocol (GOP) Specification
- docs/md/Architecture&Design/Display/ (detailed documentation)
