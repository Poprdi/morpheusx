# MorpheusX Display Stack (virtio-gpu)

Display stack for rendering and scanout in the post-ExitBootServices bare-metal environment.

**Version**: 1.0  
**Status**: DRAFT  
**Date**: January 2026

---

## Document Hierarchy

This document is subordinate to:

1. **VISION.md** — Project-wide architecture philosophy
2. **Network documentation** — Reference implementation patterns (ASM boundary, state machines)

---

## Table of Contents

1. [Design Principles](#1-design-principles)
2. [Performance Targets](#2-performance-targets)
3. [Usage](#3-usage)
4. [API Overview](#4-api-overview)
5. [Architecture](#5-architecture)
6. [Error Handling](#6-error-handling)
7. [Testing](#7-testing)
8. [UEFI Display Compatibility](#8-uefi-display-compatibility)
9. [Hardware Driver Extension Model](#9-hardware-driver-extension-model)
10. [Future](#10-future)
11. [References & Assumptions](#11-references--assumptions)

---

# 1. Design Principles

## 1.1 Core Philosophy

MorpheusX display stack operates as a **bare-metal, post-ExitBootServices exokernel runtime**:

| Principle | Description |
|-----------|-------------|
| **No firmware services** | All hardware control is manual after EBS; no UEFI GOP at runtime |
| **Deterministic control** | Explicit resource lifetimes, queue management, frame pacing |
| **Minimal surface** | Only what's needed: mode set, scanout, resource upload, fence/wait |
| **Hardware acceleration first** | Prioritize 3D-capable transports; 2D blit as degraded fallback |
| **Drop-in drivers** | Stable ASM boundary for vendor GPUs via device-specific shims |

## 1.2 Execution Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    SINGLE-THREADED FRAME LOOP                   │
│                                                                 │
│   ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐        │
│   │ Phase 1 │ → │ Phase 2 │ → │ Phase 3 │ → │ Phase 4 │ → ...  │
│   │ Cmd Buf │   │ Submit  │   │ Fence   │   │ Scanout │        │
│   │ Build   │   │ Queue   │   │ Wait    │   │ Flip    │        │
│   └─────────┘   └─────────┘   └─────────┘   └─────────┘        │
│                                                                 │
│   Target: ≤16.6ms per frame (60 FPS), Jitter: ≤2ms             │
└─────────────────────────────────────────────────────────────────┘
```

**INVARIANT**: Frame loop must complete within budget. No unbounded waits.

## 1.3 Priorities

```
Correctness > Performance > Elegance > Convenience
```

- Frame timing correctness (no tearing, proper vsync)
- Hardware acceleration throughput
- Clean abstractions
- Developer convenience

## 1.4 Language Policy

| Use Case | Language | Rationale |
|----------|----------|-----------|
| Driver logic, lifetimes, API | Rust | Safety, ownership semantics |
| MMIO/PIO, barriers, DMA setup | ASM | Explicit control, no reordering |
| Tooling, test harness | Bash/Python | Convenience for scripts |

---

# 2. Performance Targets

## 2.1 Primary Goals

| Metric | Target | Budget |
|--------|--------|--------|
| **Frame rate** | ≥60 FPS | ≤16.6 ms per frame |
| **Jitter** | ≤2 ms | Frame-to-frame variance |
| **Input latency** | ≤50 ms | Command submit → scanout visible |
| **Throughput** | ≥100 MB/s | Texture upload bandwidth |

## 2.2 Hardware Acceleration Requirements

The display stack **must** prioritize hardware-accelerated paths:

```
┌─────────────────────────────────────────────────────────────────┐
│                    DRIVER FALLBACK CHAIN                        │
│                                                                 │
│  ┌────────────────┐                                             │
│  │ Vendor GPU     │ ← Best: native 3D, full acceleration        │
│  │ (e.g., AMD,    │                                             │
│  │  Intel, NVIDIA)│                                             │
│  └───────┬────────┘                                             │
│          │ Not available                                        │
│          ▼                                                      │
│  ┌────────────────┐                                             │
│  │ virtio-gpu     │ ← Good: virgl/3D feature for hardware 3D    │
│  │ (with 3D)      │                                             │
│  └───────┬────────┘                                             │
│          │ 3D not negotiated                                    │
│          ▼                                                      │
│  ┌────────────────┐                                             │
│  │ virtio-gpu     │ ← Acceptable: 2D blit, software rasterizer  │
│  │ (2D only)      │   [DEGRADED MODE FLAG SET]                  │
│  └───────┬────────┘                                             │
│          │ virtio-gpu not available                             │
│          ▼                                                      │
│  ┌────────────────┐                                             │
│  │ Simple         │ ← Minimal: direct framebuffer writes        │
│  │ Framebuffer    │   [DEGRADED MODE FLAG SET]                  │
│  └────────────────┘                                             │
└─────────────────────────────────────────────────────────────────┘
```

**Degraded Mode Labeling**: When 3D is unavailable, set `DisplayCapabilities::DEGRADED_MODE` flag and log warning.

## 2.3 Frame Pacing & Buffering

### Swapchain Model

| Strategy | Latency | Throughput | Use Case |
|----------|---------|------------|----------|
| **Double buffering** | 1-2 frames | Good | Typical display |
| **Triple buffering** | 2-3 frames | Better | High-throughput |
| **Single buffering** | Minimal | Poor | Testing only |

### Vsync Strategy

```rust
/// Vsync configuration.
#[derive(Debug, Clone, Copy)]
pub enum VsyncMode {
    /// No vsync - immediate flip, may tear
    Off,
    /// Wait for vblank before flip - no tearing, may stutter
    On,
    /// Adaptive - vsync on if above refresh rate, off otherwise
    Adaptive,
}
```

### Frame Pacing Loop (Pseudocode)

```rust
fn frame_loop(display: &mut Display, vsync: VsyncMode) {
    let mut frame_start = tsc_read();
    
    loop {
        // Build command buffer
        let cmds = build_render_commands();
        
        // Submit to GPU
        let fence = display.submit(cmds)?;
        
        // Wait for GPU completion (bounded)
        fence.wait_timeout(FRAME_BUDGET)?;
        
        // Present to scanout
        match vsync {
            VsyncMode::On => display.present_vsync()?,
            VsyncMode::Off => display.present_immediate()?,
            VsyncMode::Adaptive => {
                let elapsed = tsc_elapsed(frame_start);
                if elapsed < FRAME_BUDGET {
                    display.present_vsync()?;
                } else {
                    display.present_immediate()?;
                }
            }
        }
        
        // Timing instrumentation
        let frame_end = tsc_read();
        record_frame_time(frame_end - frame_start);
        frame_start = frame_end;
    }
}
```

## 2.4 Throughput Optimization

| Technique | Description |
|-----------|-------------|
| **Batch submission** | Accumulate commands before queue kick |
| **Zero-copy paths** | Pinned memory for texture uploads when feasible |
| **Bulk uploads** | Staged transfer for large resources |
| **Minimize round-trips** | Fence coalescing, batched queries |

## 2.5 Latency Instrumentation

Built-in timing hooks for profiling:

```rust
/// Timing metrics collected per frame.
pub struct FrameMetrics {
    /// TSC at command buffer build start
    pub cmd_build_start: u64,
    /// TSC at queue submit
    pub queue_submit: u64,
    /// TSC at fence signal (GPU done)
    pub fence_signal: u64,
    /// TSC at scanout flip
    pub scanout_flip: u64,
}
```

Test harness collects and reports:
- P50/P95/P99 frame times
- Jitter histogram
- Dropped frame count

## 2.6 Power/Thermal Considerations

- **Prefer batching** over micro-submissions to reduce interrupt overhead
- **Avoid busy-wait** unless explicitly configured (`cfg(feature = "spin_wait")`)
- **Idle detection**: Skip render when no updates (static UI)

---

# 3. Usage

## 3.1 Basic Initialization

```rust
use morpheus_display::{Display, DisplayConfig, Mode};

// Initialize display from PCI discovery
let display = Display::init_from_pci(
    handoff.display_mmio_base,
    &mut dma_region,
    DisplayConfig {
        preferred_mode: Mode::preferred(),
        vsync: VsyncMode::On,
        swapchain_depth: 2,
    },
)?;

// Query capabilities
let caps = display.capabilities();
if caps.has_3d {
    log::info!("Hardware 3D acceleration available");
} else {
    log::warn!("Running in degraded mode (2D only)");
}

// Get current mode
let mode = display.current_mode();
log::info!("Display: {}x{} @ {}Hz", mode.width, mode.height, mode.refresh);
```

## 3.2 Resource Creation and Upload

```rust
use morpheus_display::{Resource, ResourceType, Format};

// Create 2D texture resource
let texture = display.create_resource(ResourceType::Texture2D {
    width: 1920,
    height: 1080,
    format: Format::BGRA8888,
})?;

// Upload pixel data (bulk transfer)
let pixels: &[u8] = /* BGRA pixel data */;
display.upload_resource(&texture, pixels)?;

// Alternatively, staged upload for large textures
let staging = display.create_staging_buffer(pixels.len())?;
staging.write(pixels);
display.copy_to_resource(&staging, &texture)?;
```

## 3.3 Attach Scanout

```rust
// Attach resource to scanout (make visible)
display.attach_scanout(
    0,          // scanout index (0 = primary)
    &texture,   // resource to display
    Rect { x: 0, y: 0, width: 1920, height: 1080 },
)?;

// Flush to make visible
display.flush()?;
```

## 3.4 3D Rendering Path (virtio-gpu with virgl)

```rust
// Check for 3D capability
if !display.capabilities().has_3d {
    return Err(DisplayError::No3DSupport);
}

// Create 3D context
let ctx = display.create_3d_context()?;

// Create 3D resource (render target)
let render_target = ctx.create_render_target(
    1920, 1080, 
    Format::BGRA8888,
    SampleCount::X1,
)?;

// Submit Gallium3D commands (virgl protocol)
let cmd_buf = ctx.begin_commands();
cmd_buf.clear(render_target, Color::BLACK);
cmd_buf.draw_triangles(/* vertex buffer, shader, etc. */);
let fence = cmd_buf.submit()?;

// Wait for GPU
fence.wait()?;

// Present render target to scanout
display.attach_scanout(0, &render_target, Rect::full())?;
display.flush()?;
```

## 3.5 Fencing and Synchronization

```rust
use morpheus_display::{Fence, FenceWait};

// Create fence for command tracking
let fence = display.create_fence()?;

// Submit commands with fence
display.submit_with_fence(cmds, &fence)?;

// Poll-based wait (non-blocking)
loop {
    match fence.poll()? {
        FenceWait::Signaled => break,
        FenceWait::Pending => {
            // Do other work
            do_other_work();
        }
    }
}

// Or timeout-based wait
fence.wait_timeout(timeouts.frame_budget())?;
```

## 3.6 Teardown

```rust
// Destroy resources in reverse order
display.destroy_resource(texture)?;
display.detach_scanout(0)?;

// Shutdown display
display.shutdown()?;
```

---

# 4. API Overview

## 4.1 High-Level Display API

```rust
/// Main display interface.
pub trait DisplayDriver {
    /// Initialize display from MMIO base and DMA region.
    fn init(
        mmio_base: u64,
        dma: &mut DmaRegion,
        config: DisplayConfig,
    ) -> Result<Self, DisplayError> where Self: Sized;
    
    /// Query display capabilities.
    fn capabilities(&self) -> DisplayCapabilities;
    
    /// Get/set display mode.
    fn current_mode(&self) -> Mode;
    fn set_mode(&mut self, mode: Mode) -> Result<(), DisplayError>;
    fn supported_modes(&self) -> &[Mode];
    
    /// Resource management.
    fn create_resource(&mut self, desc: ResourceType) -> Result<ResourceHandle, DisplayError>;
    fn upload_resource(&mut self, handle: &ResourceHandle, data: &[u8]) -> Result<(), DisplayError>;
    fn destroy_resource(&mut self, handle: ResourceHandle) -> Result<(), DisplayError>;
    
    /// Scanout control.
    fn attach_scanout(&mut self, idx: u32, resource: &ResourceHandle, rect: Rect) -> Result<(), DisplayError>;
    fn detach_scanout(&mut self, idx: u32) -> Result<(), DisplayError>;
    fn flush(&mut self) -> Result<(), DisplayError>;
    
    /// Synchronization.
    fn create_fence(&mut self) -> Result<Fence, DisplayError>;
    fn submit_with_fence(&mut self, cmds: &[Command], fence: &Fence) -> Result<(), DisplayError>;
    
    /// Teardown.
    fn shutdown(self) -> Result<(), DisplayError>;
}
```

## 4.2 Driver Interface Traits

### Capabilities Trait

```rust
/// Display capabilities reported by driver.
#[derive(Debug, Clone)]
pub struct DisplayCapabilities {
    /// Hardware 3D acceleration available
    pub has_3d: bool,
    /// EDID modes supported
    pub has_edid: bool,
    /// Multiple scanouts supported
    pub max_scanouts: u32,
    /// Maximum texture dimension
    pub max_texture_size: u32,
    /// Supported pixel formats
    pub formats: &'static [Format],
    /// Degraded mode (software fallback)
    pub degraded_mode: bool,
}
```

### Driver Registry Trait

```rust
/// Trait for driver registration and discovery.
pub trait GpuDriver: DisplayDriver + Sized {
    /// PCI vendor IDs this driver supports.
    fn supported_vendors() -> &'static [u16];
    
    /// PCI device IDs this driver supports.
    fn supported_devices() -> &'static [u16];
    
    /// Check if driver supports a PCI device.
    fn supports_device(vendor: u16, device: u16) -> bool {
        Self::supported_vendors().contains(&vendor) &&
        Self::supported_devices().contains(&device)
    }
    
    /// Probe device and negotiate capabilities.
    fn probe(pci_device: &PciDevice) -> Result<Self::Capabilities, ProbeError>;
    
    /// Create driver instance.
    unsafe fn create(
        mmio_base: u64,
        dma: &mut DmaRegion,
        negotiated_caps: Self::Capabilities,
    ) -> Result<Self, Self::Error>;
}
```

## 4.3 Transport Specifics (virtio-gpu)

### Feature Negotiation

```rust
/// VirtIO-GPU feature bits.
pub mod virtio_gpu_features {
    /// Virgl 3D support
    pub const VIRTIO_GPU_F_VIRGL: u32 = 1 << 0;
    /// EDID support
    pub const VIRTIO_GPU_F_EDID: u32 = 1 << 1;
    /// Resource UUID support
    pub const VIRTIO_GPU_F_RESOURCE_UUID: u32 = 1 << 2;
    /// Resource blob support
    pub const VIRTIO_GPU_F_RESOURCE_BLOB: u32 = 1 << 3;
    /// Context init support
    pub const VIRTIO_GPU_F_CONTEXT_INIT: u32 = 1 << 4;
}

/// Required features for minimal operation.
pub const REQUIRED_FEATURES: u32 = 0; // No mandatory features

/// Desired features (use if available).
pub const DESIRED_FEATURES: u32 = 
    virtio_gpu_features::VIRTIO_GPU_F_VIRGL |
    virtio_gpu_features::VIRTIO_GPU_F_EDID;
```

### Queue Layout

```
┌─────────────────────────────────────────────────────────────────┐
│                    VirtIO-GPU Queues                            │
├─────────────────────────────────────────────────────────────────┤
│  Queue 0: controlq  — GPU commands, responses                   │
│  Queue 1: cursorq   — Cursor updates (optional)                 │
└─────────────────────────────────────────────────────────────────┘
```

### Command Types

| Command | Description |
|---------|-------------|
| `VIRTIO_GPU_CMD_GET_DISPLAY_INFO` | Query display configuration |
| `VIRTIO_GPU_CMD_RESOURCE_CREATE_2D` | Create 2D resource |
| `VIRTIO_GPU_CMD_RESOURCE_UNREF` | Destroy resource |
| `VIRTIO_GPU_CMD_SET_SCANOUT` | Attach resource to scanout |
| `VIRTIO_GPU_CMD_RESOURCE_FLUSH` | Flush resource to display |
| `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D` | Upload pixel data |
| `VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING` | Attach guest memory |
| `VIRTIO_GPU_CMD_CTX_CREATE` | Create 3D context (virgl) |
| `VIRTIO_GPU_CMD_SUBMIT_3D` | Submit virgl command buffer |

---

# 5. Architecture

## 5.1 Module Data Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                     APPLICATION LAYER                           │
│              TUI Rendering, Distro Selection UI                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DISPLAY API LAYER                           │
│       DisplayDriver trait, Resource management, Fencing         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DRIVER LAYER                                │
│        VirtIO-GPU (reference), Vendor GPUs (future)             │
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │  Driver Registry                                        │   │
│   │  - PCI ID matching                                      │   │
│   │  - Capability probing                                   │   │
│   │  - Fallback ordering                                    │   │
│   └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     TRANSPORT LAYER                             │
│              VirtIO Queues, Command Submission                  │
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │  Virtqueue Management                                   │   │
│   │  - Descriptor ring                                      │   │
│   │  - Available/Used ring                                  │   │
│   │  - Fence tracking                                       │   │
│   └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     ASM LAYER (Standalone)                      │
│   Generic: MMIO, Barriers │ Device-Specific: VirtIO-GPU ops    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     HARDWARE                                    │
│              VirtIO-GPU, Intel, AMD, NVIDIA                     │
└─────────────────────────────────────────────────────────────────┘
```

## 5.2 Rust<->ASM Boundary

### Design Principles

The ASM layer provides:

1. **Guaranteed memory ordering** — Compiler cannot reorder across ASM calls
2. **Explicit barrier placement** — Developer controls when barriers execute
3. **Volatile hardware access** — MMIO reads/writes not optimized away
4. **Microsoft x64 ABI** — Compatible with UEFI calling convention

### Calling Convention (Microsoft x64)

```
Parameters:  RCX, RDX, R8, R9 (first 4 integer/pointer args)
Return:      RAX (integer), XMM0 (float)
Volatile:    RAX, RCX, RDX, R8, R9, R10, R11
Non-volatile: RBX, RBP, RDI, RSI, R12-R15
Stack:       16-byte aligned, 32-byte shadow space
```

### Memory Ordering Requirements

| Operation | Barrier | Reason |
|-----------|---------|--------|
| Write descriptor then update avail | SFENCE | Ensure descriptor visible before index |
| Read used index then read ring | LFENCE | Ensure index read before data |
| Before MMIO notify | MFENCE | Full barrier before device kick |
| After DMA complete | LFENCE | Ensure CPU sees device writes |

### ASM Function Inventory

#### Generic Functions (Reusable)

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_tsc_read` | None | `u64` | Read TSC |
| `asm_bar_sfence` | None | None | Store fence |
| `asm_bar_lfence` | None | None | Load fence |
| `asm_bar_mfence` | None | None | Full fence |
| `asm_mmio_read32` | `addr: u64` | `u32` | 32-bit MMIO read |
| `asm_mmio_write32` | `addr: u64, val: u32` | None | 32-bit MMIO write |
| `asm_mmio_read64` | `addr: u64` | `u64` | 64-bit MMIO read |
| `asm_mmio_write64` | `addr: u64, val: u64` | None | 64-bit MMIO write |

#### VirtIO-GPU Functions

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_gpu_submit_cmd` | `vq: *mut VqState, desc_idx: u16` | `u32` | Submit command |
| `asm_gpu_poll_response` | `vq: *mut VqState, out: *mut Response` | `u32` | Poll for response |
| `asm_gpu_notify` | `vq: *mut VqState` | None | Notify device |

### Rust Bindings

```rust
// src/asm/bindings.rs

extern "win64" {
    // Generic
    pub fn asm_tsc_read() -> u64;
    pub fn asm_bar_sfence();
    pub fn asm_bar_lfence();
    pub fn asm_bar_mfence();
    pub fn asm_mmio_read32(addr: u64) -> u32;
    pub fn asm_mmio_write32(addr: u64, value: u32);
    pub fn asm_mmio_read64(addr: u64) -> u64;
    pub fn asm_mmio_write64(addr: u64, value: u64);
    
    // VirtIO-GPU specific
    pub fn asm_gpu_submit_cmd(vq: *mut VirtqueueState, desc_idx: u16) -> u32;
    pub fn asm_gpu_poll_response(vq: *mut VirtqueueState, out: *mut GpuResponse) -> u32;
    pub fn asm_gpu_notify(vq: *mut VirtqueueState);
}
```

## 5.3 Resource and Queue Lifetimes

### Resource Ownership Model

```
              RESOURCE OWNERSHIP STATE MACHINE
              
          ┌─────────┐
          │ CREATED │  Allocated, no backing memory
          └────┬────┘
               │ attach_backing()
               ▼
      ┌──────────────────┐
      │   BACKED         │  Guest memory attached
      │   (Host access)  │  
      └────────┬─────────┘
               │ transfer_to_host()
               ▼
      ┌──────────────────┐
      │   UPLOADED       │  Data visible to host
      └────────┬─────────┘
               │ set_scanout()
               ▼
      ┌──────────────────┐
      │   DISPLAYED      │  Attached to scanout
      │   (Visible)      │
      └────────┬─────────┘
               │ resource_unref()
               ▼
          ┌─────────┐
          │ FREED   │
          └─────────┘
```

### Descriptor Ring Management

```rust
/// Virtqueue state for GPU commands.
#[repr(C)]
pub struct GpuVirtqueueState {
    /// Descriptor table base (bus address)
    pub desc_base: u64,
    /// Available ring base
    pub avail_base: u64,
    /// Used ring base
    pub used_base: u64,
    /// Queue size (power of 2)
    pub queue_size: u16,
    /// Queue index
    pub queue_index: u16,
    /// Notify offset
    pub notify_offset: u32,
    /// Notify address
    pub notify_addr: u64,
    /// Last seen used index
    pub last_used_idx: u16,
    /// Next available index
    pub next_avail_idx: u16,
    /// CPU pointer to descriptors
    pub desc_cpu_ptr: u64,
}
```

## 5.4 DMA/IOMMU Assumptions

### Memory Mapping Strategy

| Mode | Description | When |
|------|-------------|------|
| **Identity map** | bus_addr == cpu_addr | No IOMMU, or IOMMU in passthrough |
| **Bounce buffer** | Intermediate DMA-safe region | IOMMU with restricted addresses |

### Assumptions

```
ASSUMPTION DMA-1: DMA region allocated via PCI I/O Protocol pre-EBS
ASSUMPTION DMA-2: Bus address obtained from UEFI Map() operation
ASSUMPTION DMA-3: Post-EBS, addresses remain valid (no IOMMU reconfiguration)
ASSUMPTION DMA-4: Memory marked UC or WC for coherency
```

## 5.5 Interrupt/Poll Strategy

| Mode | Description | Configuration |
|------|-------------|---------------|
| **Poll-only** | No interrupts, explicit poll | Default post-EBS |
| **MSI-X** | Message-signaled interrupts | Optional, if configured pre-EBS |

**Default**: Poll-only (simplest, no interrupt controller setup needed post-EBS).

```rust
/// Poll for GPU completion.
/// 
/// # Contract
/// - MUST return immediately
/// - No blocking or busy-wait
fn poll_completion(&mut self) -> Option<FenceId> {
    let mut response = GpuResponse::default();
    let has_response = unsafe {
        asm_gpu_poll_response(&mut self.controlq, &mut response)
    };
    
    if has_response == 1 {
        Some(response.fence_id)
    } else {
        None
    }
}
```

## 5.6 Swapchain and Frame Pacing

### Swapchain Structure

```rust
/// Double/triple buffer swapchain.
pub struct Swapchain {
    /// Backing resources
    buffers: [ResourceHandle; MAX_SWAPCHAIN_DEPTH],
    /// Number of buffers
    depth: u32,
    /// Current front buffer index
    front: u32,
    /// Current back buffer index (for rendering)
    back: u32,
    /// Fences for each buffer
    fences: [Option<Fence>; MAX_SWAPCHAIN_DEPTH],
}

impl Swapchain {
    /// Get back buffer for rendering.
    pub fn acquire(&mut self) -> Result<&ResourceHandle, SwapchainError> {
        // Wait if back buffer still in use
        if let Some(fence) = &self.fences[self.back as usize] {
            fence.wait_timeout(ACQUIRE_TIMEOUT)?;
        }
        Ok(&self.buffers[self.back as usize])
    }
    
    /// Present back buffer, swap with front.
    pub fn present(&mut self, fence: Fence) {
        self.fences[self.back as usize] = Some(fence);
        core::mem::swap(&mut self.front, &mut self.back);
    }
}
```

---

# 6. Error Handling

## 6.1 Error Types

```rust
/// Display error enumeration.
#[derive(Debug, Clone)]
pub enum DisplayError {
    // Initialization errors
    InitFailed(InitError),
    NoDeviceFound,
    UnsupportedDevice { vendor: u16, device: u16 },
    
    // Feature errors
    FeatureNegotiationFailed,
    No3DSupport,
    ModeNotSupported(Mode),
    
    // Resource errors
    ResourceCreationFailed,
    ResourceNotFound(ResourceHandle),
    UploadFailed { resource: ResourceHandle, reason: &'static str },
    
    // Queue errors
    QueueFull,
    CommandTimeout,
    FenceTimeout,
    
    // Hardware errors
    DeviceReset,
    DeviceLost,
    InvalidResponse,
}

/// Initialization errors.
#[derive(Debug, Clone)]
pub enum InitError {
    ResetTimeout,
    PciConfigError,
    DmaAllocationFailed,
    QueueSetupFailed(u16),
}
```

## 6.2 Error Propagation

```rust
/// Result type for display operations.
pub type DisplayResult<T> = Result<T, DisplayError>;

// Error paths must be explicit
fn set_scanout_impl(...) -> DisplayResult<()> {
    let resource = self.resources.get(&handle)
        .ok_or(DisplayError::ResourceNotFound(handle))?;
    
    let cmd = build_set_scanout_cmd(scanout_idx, resource);
    
    self.submit_and_wait(cmd)
        .map_err(|e| match e {
            QueueError::Full => DisplayError::QueueFull,
            QueueError::Timeout => DisplayError::CommandTimeout,
        })?;
    
    Ok(())
}
```

## 6.3 Recovery Paths

| Error | Recovery | Degradation |
|-------|----------|-------------|
| `QueueFull` | Drain completions, retry | None |
| `CommandTimeout` | Reset queue, retry once | Log warning |
| `DeviceReset` | Full re-initialization | May lose resources |
| `No3DSupport` | Fall back to 2D blit | Set degraded flag |
| `ModeNotSupported` | Use closest supported mode | Log warning |

## 6.4 Degraded Mode Handling

```rust
/// Check and handle degraded mode.
fn handle_degraded_mode(caps: &DisplayCapabilities) {
    if caps.degraded_mode {
        log::warn!("Display running in degraded mode (no hardware 3D)");
        log::warn!("Performance may be significantly reduced");
        
        // Adjust frame rate target
        set_target_fps(30); // Lower target for software path
    }
}
```

---

# 7. Testing

## 7.1 QEMU Test Configuration

### virtio-gpu with 3D (virgl)

```bash
#!/bin/bash
# Run MorpheusX in QEMU with virtio-gpu (3D enabled)

qemu-system-x86_64 \
    -enable-kvm \
    -m 4G \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=esp.img,format=raw \
    -device virtio-gpu-pci,virgl=on \
    -display gtk,gl=on \
    -serial stdio \
    -no-reboot
```

### virtio-vga (2D fallback)

```bash
#!/bin/bash
# Run MorpheusX in QEMU with virtio-vga (2D only)

qemu-system-x86_64 \
    -enable-kvm \
    -m 2G \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=esp.img,format=raw \
    -device virtio-vga \
    -display gtk \
    -serial stdio \
    -no-reboot
```

### Headless Testing

```bash
#!/bin/bash
# Run MorpheusX in QEMU headless (for CI)

qemu-system-x86_64 \
    -enable-kvm \
    -m 2G \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=esp.img,format=raw \
    -device virtio-gpu-pci \
    -display none \
    -serial stdio \
    -no-reboot
```

## 7.2 Test Scripts

### Display Init Test

```bash
#!/bin/bash
# test-display-init.sh

set -e

echo "Testing display initialization..."

timeout 60 qemu-system-x86_64 \
    -enable-kvm \
    -m 2G \
    -bios /usr/share/OVMF/OVMF_CODE.fd \
    -drive file=esp.img,format=raw \
    -device virtio-gpu-pci \
    -display none \
    -serial stdio 2>&1 | tee /tmp/display-test.log

# Check for success markers in log
grep -q "Display initialized" /tmp/display-test.log
grep -q "Mode set:" /tmp/display-test.log

echo "Display init test PASSED"
```

## 7.3 CI Integration

```yaml
# .github/workflows/display-test.yml
name: Display Stack Tests

on: [push, pull_request]

jobs:
  display-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install QEMU
        run: |
          sudo apt-get update
          sudo apt-get install -y qemu-system-x86 ovmf
      
      - name: Build MorpheusX
        run: cargo build --release
      
      - name: Run display tests
        run: ./scripts/test-display.sh
```

## 7.4 Latency/Throughput Instrumentation

```rust
/// Test harness for frame timing measurement.
pub fn benchmark_frame_times(display: &mut Display, iterations: u32) -> FrameStats {
    let mut times = Vec::with_capacity(iterations as usize);
    
    for _ in 0..iterations {
        let start = unsafe { asm_tsc_read() };
        
        // Simulate frame
        let resource = display.create_resource(/* test params */).unwrap();
        display.upload_resource(&resource, &test_data).unwrap();
        display.attach_scanout(0, &resource, Rect::full()).unwrap();
        display.flush().unwrap();
        
        let end = unsafe { asm_tsc_read() };
        times.push(end - start);
        
        display.destroy_resource(resource).unwrap();
    }
    
    compute_stats(&times)
}

/// Frame timing statistics.
pub struct FrameStats {
    pub min_ticks: u64,
    pub max_ticks: u64,
    pub p50_ticks: u64,
    pub p95_ticks: u64,
    pub p99_ticks: u64,
    pub mean_ticks: u64,
    pub jitter_ticks: u64,
}
```

---

# 8. UEFI Display Compatibility

## 8.1 Overview

The display stack must maintain **backward compatibility** with code relying on UEFI Graphics Output Protocol (GOP). A compatibility shim maps legacy UEFI display APIs to the new bare-metal driver.

## 8.2 Compatibility Shim Design

```
┌─────────────────────────────────────────────────────────────────┐
│                     LEGACY CALLERS                              │
│           (existing code using UEFI GOP-style API)              │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                 UEFI COMPATIBILITY SHIM                         │
│                                                                 │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │  uefi_compat::GraphicsOutput                            │   │
│   │  - query_mode()  → driver.supported_modes()             │   │
│   │  - set_mode()    → driver.set_mode()                    │   │
│   │  - blt()         → driver.upload_resource() + flush()   │   │
│   └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     NEW DISPLAY DRIVER                          │
│              (virtio-gpu or vendor driver)                      │
└─────────────────────────────────────────────────────────────────┘
```

## 8.3 Feature Flags

```toml
# Cargo.toml
[features]
default = []
uefi-compat = []  # Enable UEFI GOP compatibility layer
```

```rust
// Conditional compilation
#[cfg(feature = "uefi-compat")]
pub mod uefi_compat;
```

## 8.4 API Equivalence Table

| UEFI GOP Function | Shim Mapping | Notes |
|-------------------|--------------|-------|
| `QueryMode()` | `driver.supported_modes()` | Returns mode list |
| `SetMode()` | `driver.set_mode()` | Mode change |
| `Blt(EfiBltVideoFill)` | `memset` + `upload` + `flush` | Fill rectangle |
| `Blt(EfiBltVideoToBltBuffer)` | `download_resource` | Read from framebuffer |
| `Blt(EfiBltBufferToVideo)` | `upload_resource` + `flush` | Write to framebuffer |
| `Blt(EfiBltVideoToVideo)` | `copy_resource` + `flush` | Copy rectangle |

## 8.5 Behavior Differences

| Behavior | UEFI GOP | New Driver |
|----------|----------|------------|
| Initialization | Automatic from firmware | Explicit `init_from_pci()` |
| Mode enumeration | Via GOP protocol | Via driver capability query |
| Memory model | Framebuffer linear address | Resource-based with handles |
| Synchronization | Implicit (blocking) | Explicit fencing |
| Teardown | None (firmware manages) | Explicit `shutdown()` |

## 8.6 Pre-EBS vs Post-EBS

```rust
/// Display initialization path.
pub fn init_display(boot_phase: BootPhase, handoff: &BootHandoff) -> DisplayResult<Box<dyn DisplayDriver>> {
    match boot_phase {
        BootPhase::PreEBS => {
            // Use UEFI GOP if available
            #[cfg(feature = "uefi-compat")]
            if let Some(gop) = get_uefi_gop() {
                return Ok(Box::new(UefiGopDriver::new(gop)));
            }
            Err(DisplayError::NoDeviceFound)
        }
        BootPhase::PostEBS => {
            // Use bare-metal driver
            let driver = VirtioGpuDriver::init(
                handoff.display_mmio_base,
                &mut handoff.dma_region,
                DisplayConfig::default(),
            )?;
            Ok(Box::new(driver))
        }
    }
}
```

## 8.7 Migration Guidance

### For Callers Using Legacy API

1. **Add feature flag dependency**:
   ```toml
   morpheus_display = { features = ["uefi-compat"] }
   ```

2. **Gradual migration**:
   ```rust
   // Phase 1: Use shim
   let display = uefi_compat::GraphicsOutput::new(driver);
   display.blt(...);  // Legacy API
   
   // Phase 2: Direct driver usage
   let display = driver;
   display.upload_resource(...);  // New API
   ```

3. **Deprecation timeline**:
   - v1.0: Both APIs supported
   - v2.0: Legacy API deprecated with warnings
   - v3.0: Legacy API removed

---

# 9. Hardware Driver Extension Model

## 9.1 Overview

The display stack is architected for **drop-in drivers** via device-specific shims implementing a common minimal capability interface.

## 9.2 Driver Packaging

```
display/
├── src/
│   ├── lib.rs                 # Crate root
│   ├── driver/
│   │   ├── mod.rs             # Driver registry
│   │   ├── traits.rs          # GpuDriver trait
│   │   ├── virtio_gpu.rs      # VirtIO-GPU (reference)
│   │   ├── intel/             # Intel GPU (future)
│   │   │   ├── mod.rs
│   │   │   └── gen9.rs        # Generation-specific
│   │   └── amd/               # AMD GPU (future)
│   │       └── mod.rs
│   └── asm/
│       ├── mod.rs             # ASM bindings
│       └── types.rs           # Shared types
├── asm/
│   ├── generic.s              # Generic ASM (shared)
│   ├── virtio_gpu.s           # VirtIO-GPU ASM
│   ├── intel.s                # Intel ASM (future)
│   └── amd.s                  # AMD ASM (future)
└── Cargo.toml
```

## 9.3 Registration and Discovery

```rust
/// Driver registry with priority ordering.
pub struct DriverRegistry {
    drivers: Vec<Box<dyn GpuDriverFactory>>,
}

impl DriverRegistry {
    /// Register a driver factory.
    pub fn register<D: GpuDriver + 'static>(&mut self) {
        self.drivers.push(Box::new(TypedDriverFactory::<D>::new()));
    }
    
    /// Find driver for PCI device (priority order).
    pub fn find_driver(&self, vendor: u16, device: u16) -> Option<&dyn GpuDriverFactory> {
        // Priority: vendor-specific > virtio-gpu > framebuffer
        for driver in &self.drivers {
            if driver.supports(vendor, device) {
                return Some(driver.as_ref());
            }
        }
        None
    }
}

/// Initialize registry with all supported drivers.
pub fn init_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    
    // Register in priority order (highest first)
    #[cfg(feature = "driver-intel")]
    registry.register::<IntelGpuDriver>();
    
    #[cfg(feature = "driver-amd")]
    registry.register::<AmdGpuDriver>();
    
    // VirtIO-GPU always available as baseline
    registry.register::<VirtioGpuDriver>();
    
    // Simple framebuffer as last resort
    registry.register::<SimpleFramebufferDriver>();
    
    registry
}
```

## 9.4 Capability Negotiation

```rust
/// Capability probing result.
pub struct ProbeResult {
    /// Driver name
    pub name: &'static str,
    /// Detected capabilities
    pub capabilities: DisplayCapabilities,
    /// Priority (higher = preferred)
    pub priority: u32,
}

/// Probe all compatible drivers, return best match.
pub fn probe_best_driver(
    registry: &DriverRegistry,
    pci_device: &PciDevice,
) -> Result<ProbeResult, ProbeError> {
    let mut results = Vec::new();
    
    for driver in registry.iter_compatible(pci_device.vendor, pci_device.device) {
        if let Ok(probe) = driver.probe(pci_device) {
            results.push(probe);
        }
    }
    
    // Sort by priority (highest first), then by capability score
    results.sort_by(|a, b| {
        b.priority.cmp(&a.priority)
            .then_with(|| capability_score(&b.capabilities).cmp(&capability_score(&a.capabilities)))
    });
    
    results.into_iter().next().ok_or(ProbeError::NoCompatibleDriver)
}

fn capability_score(caps: &DisplayCapabilities) -> u32 {
    let mut score = 0;
    if caps.has_3d { score += 100; }
    if caps.has_edid { score += 10; }
    score += caps.max_scanouts;
    score
}
```

## 9.5 Fallback Chain

```
┌─────────────────────────────────────────────────────────────────┐
│                    DRIVER FALLBACK CHAIN                        │
│                                                                 │
│  1. Vendor-specific driver (Intel, AMD, NVIDIA)                 │
│     └─ Requires: Matching PCI ID, ASM shim                      │
│     └─ Provides: Full hardware acceleration                     │
│                                                                 │
│  2. virtio-gpu with virgl                                       │
│     └─ Requires: VirtIO GPU device, 3D feature negotiated       │
│     └─ Provides: Hardware 3D via virgl                          │
│                                                                 │
│  3. virtio-gpu 2D only                                          │
│     └─ Requires: VirtIO GPU device                              │
│     └─ Provides: 2D blit, software rasterizer                   │
│     └─ Sets: DEGRADED_MODE flag                                 │
│                                                                 │
│  4. Simple framebuffer                                          │
│     └─ Requires: Framebuffer address from handoff               │
│     └─ Provides: Direct pixel writes                            │
│     └─ Sets: DEGRADED_MODE flag                                 │
│                                                                 │
│  5. No display (error)                                          │
│     └─ Returns: DisplayError::NoDeviceFound                     │
└─────────────────────────────────────────────────────────────────┘
```

## 9.6 Drop-In Driver Procedure

To add a new GPU driver:

### Step 1: Implement ASM Layer

```nasm
; asm/vendor.s
; Vendor-specific MMIO operations

global asm_vendor_init
global asm_vendor_set_mode
global asm_vendor_submit_cmd

asm_vendor_init:
    ; Vendor-specific initialization
    ret

asm_vendor_set_mode:
    ; Mode set sequence
    ret

asm_vendor_submit_cmd:
    ; Command submission
    ret
```

### Step 2: Implement Driver Struct

```rust
// src/driver/vendor.rs

pub struct VendorGpuDriver {
    mmio_base: u64,
    // ... driver state
}

impl DisplayDriver for VendorGpuDriver {
    // Implement all required methods
}

impl GpuDriver for VendorGpuDriver {
    fn supported_vendors() -> &'static [u16] { &[0xVEND] }
    fn supported_devices() -> &'static [u16] { &[0xDEV1, 0xDEV2] }
    
    fn probe(pci_device: &PciDevice) -> Result<ProbeResult, ProbeError> {
        // Probe hardware capabilities
    }
    
    unsafe fn create(...) -> Result<Self, Self::Error> {
        // Initialize driver
    }
}
```

### Step 3: Register in Registry

```rust
// In init_driver_registry()
#[cfg(feature = "driver-vendor")]
registry.register::<VendorGpuDriver>();
```

### Step 4: Update Build

```rust
// build.rs
let asm_files = [
    "asm/generic.s",
    "asm/virtio_gpu.s",
    #[cfg(feature = "driver-vendor")]
    "asm/vendor.s",
];
```

---

# 10. Future

## 10.1 Vendor Driver Roadmap

| Vendor | Priority | Status | Notes |
|--------|----------|--------|-------|
| VirtIO-GPU | P0 | Reference | Baseline implementation |
| Intel Gen9+ | P1 | Planned | i915-compatible |
| AMD RDNA | P2 | Planned | AMDGPU-compatible |
| NVIDIA | P3 | Research | Requires reverse engineering or Nouveau |

## 10.2 Compositor and Cursor

- **Compositor**: Multi-window rendering with damage tracking
- **Hardware cursor**: CURSOR_SET/CURSOR_MOVE via virtio-gpu cursorq
- **Overlay planes**: For video playback, compositor optimization

## 10.3 Multi-Scanout

```rust
/// Multi-display configuration.
pub struct MultiDisplayConfig {
    /// Number of active displays
    pub display_count: u32,
    /// Per-display configuration
    pub displays: [DisplayConfig; MAX_DISPLAYS],
    /// Layout (extended, mirrored, etc.)
    pub layout: DisplayLayout,
}
```

## 10.4 ARM64 Parity

| Feature | x86_64 | ARM64 | Notes |
|---------|--------|-------|-------|
| VirtIO-GPU | ✅ | ✅ | Same driver |
| Vendor GPU | Intel/AMD | Mali/Adreno | Different ASM |
| Memory barriers | MFENCE | DMB | Architecture-specific |
| PCI enumeration | Standard | ECAM | Same conceptually |

## 10.5 Performance Optimizations

### Batching

```rust
/// Command buffer with batched submission.
pub struct CommandBatch {
    commands: Vec<Command>,
    max_batch_size: usize,
}

impl CommandBatch {
    /// Add command to batch.
    pub fn push(&mut self, cmd: Command) {
        self.commands.push(cmd);
    }
    
    /// Submit entire batch with single queue kick.
    pub fn submit(self, driver: &mut impl DisplayDriver) -> Result<Fence, DisplayError> {
        driver.submit_batch(&self.commands)
    }
}
```

### Zero-Copy Texture Upload

```rust
/// Zero-copy texture upload (when supported).
pub fn upload_zero_copy(
    driver: &mut impl DisplayDriver,
    resource: &ResourceHandle,
    pinned_memory: &PinnedMemory,
) -> Result<(), DisplayError> {
    // Attach guest memory directly to resource
    driver.attach_backing(resource, pinned_memory.bus_addr(), pinned_memory.len())?;
    
    // Transfer uses DMA from pinned memory
    driver.transfer_to_host(resource, Rect::full())?;
    
    Ok(())
}
```

### Fence Scheduling

```rust
/// Coalesce multiple fences into single wait.
pub fn wait_all_fences(fences: &[Fence], timeout: Duration) -> Result<(), FenceError> {
    let deadline = Instant::now() + timeout;
    
    for fence in fences {
        let remaining = deadline.saturating_duration_since(Instant::now());
        fence.wait_timeout(remaining)?;
    }
    
    Ok(())
}
```

## 10.6 Dynamic Power Management

```rust
/// Power state management.
pub enum PowerState {
    /// Full performance
    Active,
    /// Reduced clocks, lower power
    Idle,
    /// Minimal power, display off
    Standby,
}

impl DisplayDriver for VirtioGpuDriver {
    fn set_power_state(&mut self, state: PowerState) -> Result<(), DisplayError> {
        match state {
            PowerState::Active => { /* Resume full clocks */ }
            PowerState::Idle => { /* Reduce refresh rate, dim backlight */ }
            PowerState::Standby => { /* DPMS off */ }
        }
        Ok(())
    }
}
```

---

# 11. References & Assumptions

## 11.1 Specifications

| Document | Version | Usage |
|----------|---------|-------|
| VirtIO Specification | 1.2 | virtio-gpu device model |
| Virgl Protocol | N/A | 3D command encoding |
| UEFI Specification | 2.10 | GOP compatibility reference |
| PCI Express Base | 5.0 | BAR mapping, capability parsing |

## 11.2 QEMU Behavior Assumptions

```
ASSUMPTION QEMU-1: virtio-gpu device exposed at standard PCI location
ASSUMPTION QEMU-2: virgl=on enables VIRTIO_GPU_F_VIRGL feature
ASSUMPTION QEMU-3: Display info returns at least one valid scanout
ASSUMPTION QEMU-4: Resource IDs are unique and stable until unref
```

## 11.3 Hardware Assumptions

```
ASSUMPTION HW-1: PCI configuration space accessible via standard ECAM
ASSUMPTION HW-2: MMIO BARs are memory-mapped, not I/O ports
ASSUMPTION HW-3: DMA addresses below 4GB (32-bit BARs)
ASSUMPTION HW-4: No IOMMU or IOMMU in identity/passthrough mode
```

## 11.4 Runtime Assumptions

```
ASSUMPTION RT-1: Single-threaded execution (no concurrent access)
ASSUMPTION RT-2: No interrupts enabled post-EBS (poll-only)
ASSUMPTION RT-3: TSC available and calibrated for timing
ASSUMPTION RT-4: Stack allocated and valid for duration
```

---

*End of Display Stack Documentation*
