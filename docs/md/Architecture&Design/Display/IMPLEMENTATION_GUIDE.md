# MorpheusX Display Stack Implementation Guide

**Version**: 1.0  
**Status**: AUTHORITATIVE  
**Date**: January 2026  

---

## Document Hierarchy

This document provides concrete implementation guidance. It is subordinate to:

1. **DISPLAY_ASM_RUST_ABI_CONTRACT.md** — Frozen ABI specification
2. **UEFI_COMPAT_LAYER.md** — UEFI compatibility requirements
3. **HARDWARE_EXTENSION_GUIDE.md** — Hardware extension architecture

Where conflicts exist, the hierarchy above determines precedence.

---

## Table of Contents

1. [Overview & Architecture](#1-overview--architecture)
2. [ASM Layer Specification](#2-asm-layer-specification)
3. [DMA & GPU Resource Management](#3-dma--gpu-resource-management)
4. [VirtIO-GPU Driver Implementation](#4-virtio-gpu-driver-implementation)
5. [Performance & Frame Pacing](#5-performance--frame-pacing)
6. [State Machines](#6-state-machines)
7. [Main Loop & Execution Model](#7-main-loop--execution-model)
8. [Boot Integration](#8-boot-integration)
9. [Driver Abstraction Layer](#9-driver-abstraction-layer)
10. [Testing & Validation](#10-testing--validation)

---

# 1. Overview & Architecture

## 1.1 System Context

MorpheusX display stack operates in a **post-ExitBootServices bare-metal environment**:

| Constraint | Implication |
|------------|-------------|
| No UEFI runtime services for graphics | Must implement full stack |
| Single-core, no interrupts | Poll-driven execution only |
| No heap allocator (optional) | Pre-allocated buffers |
| No threads, no async runtime | Cooperative state machines |
| Direct hardware access | ASM layer for MMIO/PIO |

## 1.2 Performance Goals

| Goal | Target | Mechanism |
|------|--------|-----------|
| Frame rate | ≥60 FPS | Hardware acceleration required |
| Frame budget | ≤16.6 ms | Per-frame time limit |
| Jitter | ≤2 ms | Double/triple buffering, explicit vsync |
| Throughput | Batched submission | Minimize CPU-GPU round trips |
| Latency | Instrumented | Timing hooks for submit, fence, swap |

## 1.3 Execution Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    SINGLE-THREADED POLL LOOP                    │
│                                                                 │
│   ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐        │
│   │ Phase 1 │ → │ Phase 2 │ → │ Phase 3 │ → │ Phase 4 │ → ...  │
│   │ Fence   │   │ Command │   │ Execute │   │ Present │        │
│   │ Poll    │   │ Build   │   │ Submit  │   │ Swap    │        │
│   └─────────┘   └─────────┘   └─────────┘   └─────────┘        │
│                                                                 │
│   Target: ≤16.6ms per frame, Jitter: ≤2ms                      │
└─────────────────────────────────────────────────────────────────┘
```

**INVARIANT**: No function may block. All operations return immediately with status.

## 1.4 Layered Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     APPLICATION LAYER                           │
│          3D Rendering State Machine, UI Composition             │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DISPLAY API LAYER                           │
│        DisplayDriver trait, Frame Pacing, Vsync Strategy        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DRIVER LAYER                                │
│     VirtIO-GPU (baseline), Vendor GPUs (future drop-ins)        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     ASM LAYER (Standalone)                      │
│   Generic: TSC, Barriers, MMIO │ GPU-Specific: VirtIO-GPU ops   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     HARDWARE                                    │
│      VirtIO-GPU, AMD GPU, Intel GPU, NVIDIA GPU (future)        │
└─────────────────────────────────────────────────────────────────┘
```

## 1.5 Key Design Decisions

| Decision | Rationale | Reference |
|----------|-----------|-----------|
| Standalone ASM (no inline) | Compiler cannot reorder; explicit barrier control | ABI Contract §1.2 |
| Poll-driven (no interrupts) | Simplicity; UEFI interrupt state undefined post-EBS | §7.1.3 |
| State machines (no blocking) | Single thread cannot wait; must yield | §6 |
| Fire-and-forget command submit | Completion collected separately; no send-wait | §5.5.2 |
| Pre-allocated GPU resources | No allocator dependency; deterministic | §3.1 |
| Double/triple buffering | Frame pacing, reduced tearing | §5.2 |

## 1.6 File Organization

```
display/
├── asm/
│   ├── generic.s           # TSC, barriers, MMIO (shared with network)
│   └── virtio_gpu.s        # VirtIO-GPU specific operations
├── src/
│   ├── lib.rs              # Crate root, re-exports
│   ├── asm/
│   │   ├── mod.rs          # ASM module
│   │   ├── bindings.rs     # extern "win64" declarations
│   │   └── types.rs        # GpuCommand, ResourceState structs
│   ├── resource/
│   │   ├── mod.rs          # Resource module
│   │   ├── buffer.rs       # GPU buffer management
│   │   └── pool.rs         # Resource pool management
│   ├── device/
│   │   ├── mod.rs          # DisplayDriver trait
│   │   ├── factory.rs      # Auto-detection, UnifiedDisplayDevice
│   │   └── virtio_gpu.rs   # VirtIO-GPU driver
│   ├── frame/
│   │   ├── mod.rs          # Frame management
│   │   ├── pacing.rs       # Frame pacing, vsync
│   │   └── metrics.rs      # Latency instrumentation
│   ├── compat/
│   │   ├── mod.rs          # UEFI compatibility
│   │   └── shim.rs         # Legacy API shim
│   └── mainloop.rs         # Display main loop integration
├── build.rs                # NASM assembly compilation
└── Cargo.toml
```

## 1.7 Forbidden Patterns

These patterns are **strictly prohibited** anywhere in the codebase:

```rust
// ❌ FORBIDDEN: Blocking wait for fence
while !fence_signaled(fence_id) {
    spin_loop();
}

// ❌ FORBIDDEN: Blocking wait for vsync
while !vsync_ready() {
    delay_us(100);
}

// ❌ FORBIDDEN: Inline assembly for hardware operations
// Don't use inline asm! - use standalone ASM functions instead
// unsafe { asm!("mfence"); }  // BAD: Use asm_bar_mfence() instead

// ❌ FORBIDDEN: Hardcoded frame timing
const FRAME_TIME_US: u64 = 16667;  // Use calibrated TSC value

// ❌ FORBIDDEN: Synchronous present
fn present(&mut self) {
    self.submit_flip();
    while !self.flip_complete() { }  // NEVER WAIT
}
```

## 1.8 Required Patterns

These patterns **must be used** for correctness:

```rust
// ✅ REQUIRED: Non-blocking fence poll
pub fn poll_fence(&mut self, fence_id: FenceId) -> Result<bool, DisplayError> {
    let signaled = unsafe { asm_gpu_poll_fence(&self.ctrl_queue, fence_id) };
    Ok(signaled != 0)
}

// ✅ REQUIRED: Fire-and-forget command submit
pub fn submit_command(&mut self, cmd: &GpuCommand) -> Result<(), DisplayError> {
    if !self.can_submit() {
        return Err(DisplayError::QueueFull);
    }
    unsafe { asm_gpu_submit_cmd(&mut self.ctrl_queue, cmd) };
    Ok(())  // Completion collected in main loop
}

// ✅ REQUIRED: Timeout as observation
let elapsed = now_tsc.wrapping_sub(frame_start_tsc);
if elapsed > frame_budget_ticks {
    // Frame budget exceeded - log, continue
    metrics.record_frame_overrun(elapsed);
}

// ✅ REQUIRED: ASM for all hardware access
let value = unsafe { asm_mmio_read32(register_addr) };
unsafe { asm_bar_mfence(); }
unsafe { asm_mmio_write32(notify_addr, queue_index) };
```

---

# 2. ASM Layer Specification

## 2.1 Design Principles

The ASM layer provides:

1. **Guaranteed memory ordering** — Compiler cannot reorder across ASM calls
2. **Explicit barrier placement** — Developer controls when barriers execute
3. **Volatile hardware access** — MMIO reads/writes not optimized away
4. **Microsoft x64 ABI** — Compatible with UEFI calling convention

**Reference**: DISPLAY_ASM_RUST_ABI_CONTRACT.md (FROZEN v1.0)

## 2.2 Function Inventory

### 2.2.1 Generic Functions (Shared with Network)

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_tsc_read` | None | `RAX: u64` | Read TSC (~40 cycles) |
| `asm_tsc_read_serialized` | None | `RAX: u64` | TSC with CPUID serialize (~200 cycles) |
| `asm_bar_sfence` | None | None | Store fence |
| `asm_bar_lfence` | None | None | Load fence |
| `asm_bar_mfence` | None | None | Full memory fence |
| `asm_mmio_read32` | `RCX: addr` | `RAX: u32` | 32-bit MMIO read |
| `asm_mmio_write32` | `RCX: addr, RDX: val` | None | 32-bit MMIO write |

### 2.2.2 VirtIO-GPU Functions

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_gpu_submit_cmd` | `RCX: *CtrlQueue, RDX: *GpuCmd` | `RAX: 0=ok, 1=full` | Submit GPU command |
| `asm_gpu_poll_response` | `RCX: *CtrlQueue` | `RAX: 0=empty, 1=resp` | Poll for command response |
| `asm_gpu_submit_cursor` | `RCX: *CursorQueue, RDX: *CursorCmd` | `RAX: 0=ok, 1=full` | Submit cursor update |
| `asm_gpu_notify` | `RCX: *VqState` | None | Notify device (mfence + MMIO) |
| `asm_gpu_reset` | `RCX: mmio_base` | `RAX: 0=ok, 1=timeout` | Reset device (≤100ms) |
| `asm_gpu_set_status` | `RCX: mmio_base, RDX: status` | None | Write status register |
| `asm_gpu_get_status` | `RCX: mmio_base` | `RAX: u8` | Read status register |
| `asm_gpu_read_features` | `RCX: mmio_base` | `RAX: u64` | Read feature bits |
| `asm_gpu_write_features` | `RCX: mmio_base, RDX: features` | None | Write feature bits |
| `asm_gpu_get_display_info` | `RCX: mmio_base, RDX: *DisplayInfo` | `RAX: 0=ok, 1=err` | Read display info |
| `asm_gpu_create_resource` | `RCX: *CtrlQueue, RDX: *CreateCmd` | `RAX: 0=ok, 1=full` | Create 2D resource |
| `asm_gpu_attach_backing` | `RCX: *CtrlQueue, RDX: *AttachCmd` | `RAX: 0=ok, 1=full` | Attach backing pages |
| `asm_gpu_set_scanout` | `RCX: *CtrlQueue, RDX: *ScanoutCmd` | `RAX: 0=ok, 1=full` | Set scanout resource |
| `asm_gpu_transfer_to_host` | `RCX: *CtrlQueue, RDX: *TransferCmd` | `RAX: 0=ok, 1=full` | Transfer to host |
| `asm_gpu_resource_flush` | `RCX: *CtrlQueue, RDX: *FlushCmd` | `RAX: 0=ok, 1=full` | Flush resource to display |
| `asm_gpu_create_fence` | `RCX: *CtrlQueue, RDX: fence_id` | `RAX: 0=ok, 1=full` | Create GPU fence |
| `asm_gpu_poll_fence` | `RCX: *CtrlQueue, RDX: fence_id` | `RAX: 0=pending, 1=signaled` | Poll fence status |

### 2.2.3 3D Functions (virgl/3D Feature)

| Symbol | Parameters | Returns | Purpose |
|--------|------------|---------|---------|
| `asm_gpu_ctx_create` | `RCX: *CtrlQueue, RDX: ctx_id` | `RAX: 0=ok, 1=err` | Create 3D context |
| `asm_gpu_ctx_destroy` | `RCX: *CtrlQueue, RDX: ctx_id` | `RAX: 0=ok, 1=err` | Destroy 3D context |
| `asm_gpu_submit_3d` | `RCX: *CtrlQueue, RDX: *Cmd3D, R8: len` | `RAX: 0=ok, 1=full` | Submit 3D command buffer |
| `asm_gpu_create_resource_3d` | `RCX: *CtrlQueue, RDX: *Create3DCmd` | `RAX: 0=ok, 1=full` | Create 3D resource |

## 2.3 Calling Convention (Microsoft x64)

```
Parameters:  RCX, RDX, R8, R9 (first 4 integer/pointer args)
Return:      RAX (integer), XMM0 (float)
Volatile:    RAX, RCX, RDX, R8, R9, R10, R11
Non-volatile: RBX, RBP, RDI, RSI, R12-R15
Stack:       16-byte aligned, 32-byte shadow space
```

## 2.4 Memory Barrier Contracts

### Command Submit Sequence (asm_gpu_submit_cmd)

```asm
; Internal barrier sequence:
; 1. Write descriptor (addr, len, flags, next)
; 2. SFENCE - ensure descriptor visible
; 3. Write avail.ring[avail.idx & mask] = desc_idx
; 4. SFENCE - ensure ring entry visible  
; 5. Write avail.idx += 1
; 6. MFENCE - full barrier before notify decision
; 7. IF notify needed: MMIO write to notify register
```

### Response Poll Sequence (asm_gpu_poll_response)

```asm
; Internal barrier sequence:
; 1. Read used.idx (volatile)
; 2. Compare with last_seen_used_idx
; 3. If equal: return 0 (no response)
; 4. LFENCE - ensure index read completes
; 5. Read used.ring[last_seen & mask] → (desc_idx, len)
; 6. LFENCE - ensure ring entry read before response access
; 7. Return 1, response available
```

## 2.5 Rust Bindings

```rust
// src/asm/bindings.rs

//! ASM function bindings for MorpheusX display stack.
//! 
//! All functions use Microsoft x64 calling convention (extern "win64").
//! SAFETY: See individual function documentation for preconditions.

use crate::asm::types::{CtrlQueueState, GpuCommand, DisplayInfo};

// ═══════════════════════════════════════════════════════════════
// VIRTIO-GPU FUNCTIONS
// ═══════════════════════════════════════════════════════════════

extern "win64" {
    /// Submit GPU command to control queue.
    /// 
    /// # Safety
    /// - `ctrl_queue` must point to valid, initialized CtrlQueueState
    /// - `cmd` must point to valid GpuCommand
    /// - Command buffer content must be properly initialized
    /// 
    /// # Returns
    /// - 0: Success (command submitted)
    /// - 1: Queue full (retry after collecting responses)
    pub fn asm_gpu_submit_cmd(
        ctrl_queue: *mut CtrlQueueState,
        cmd: *const GpuCommand,
    ) -> u32;

    /// Poll control queue for command response.
    /// 
    /// # Safety
    /// - `ctrl_queue` must point to valid CtrlQueueState
    /// 
    /// # Returns
    /// - 0: No response available
    /// - 1: Response ready (check response buffer)
    pub fn asm_gpu_poll_response(ctrl_queue: *mut CtrlQueueState) -> u32;

    /// Notify device that commands are available.
    /// 
    /// # Safety
    /// - `vq_state` must point to valid queue state with notify_addr set
    /// 
    /// # Note
    /// Includes mfence before MMIO write.
    pub fn asm_gpu_notify(vq_state: *mut CtrlQueueState);

    /// Reset VirtIO-GPU device.
    /// 
    /// # Safety
    /// - `mmio_base` must be valid VirtIO MMIO base address
    /// 
    /// # Returns
    /// - 0: Reset successful
    /// - 1: Timeout (device did not reset within 100ms) - FATAL
    pub fn asm_gpu_reset(mmio_base: u64) -> u32;

    /// Write VirtIO device status register.
    pub fn asm_gpu_set_status(mmio_base: u64, status: u8);

    /// Read VirtIO device status register.
    pub fn asm_gpu_get_status(mmio_base: u64) -> u8;

    /// Read VirtIO device feature bits (64-bit).
    pub fn asm_gpu_read_features(mmio_base: u64) -> u64;

    /// Write driver-accepted feature bits.
    pub fn asm_gpu_write_features(mmio_base: u64, features: u64);

    /// Get display information from device.
    /// 
    /// # Safety
    /// - `display_info` must point to valid DisplayInfo buffer
    /// 
    /// # Returns
    /// - 0: Success
    /// - 1: Error
    pub fn asm_gpu_get_display_info(
        mmio_base: u64, 
        display_info: *mut DisplayInfo
    ) -> u32;

    /// Create fence for synchronization.
    /// 
    /// # Returns
    /// - 0: Fence created successfully
    /// - 1: Queue full
    pub fn asm_gpu_create_fence(
        ctrl_queue: *mut CtrlQueueState,
        fence_id: u64,
    ) -> u32;

    /// Poll fence status.
    /// 
    /// # Returns
    /// - 0: Fence pending
    /// - 1: Fence signaled
    pub fn asm_gpu_poll_fence(
        ctrl_queue: *mut CtrlQueueState,
        fence_id: u64,
    ) -> u32;
}
```

## 2.6 Shared Types

```rust
// src/asm/types.rs

/// Control queue state passed to ASM functions.
/// Must match ASM layout exactly.
#[repr(C)]
pub struct CtrlQueueState {
    /// Base address of descriptor table (bus address for device)
    pub desc_base: u64,
    /// Base address of available ring
    pub avail_base: u64,
    /// Base address of used ring
    pub used_base: u64,
    /// Queue size (number of descriptors)
    pub queue_size: u16,
    /// Queue index (0=control, 1=cursor)
    pub queue_index: u16,
    /// Padding for alignment
    pub _pad: u32,
    /// MMIO address for queue notification
    pub notify_addr: u64,
    /// Last seen used index (for polling)
    pub last_used_idx: u16,
    /// Next available index (for submission)
    pub next_avail_idx: u16,
    /// Padding
    pub _pad2: u32,
    /// CPU pointer to command buffer
    pub cmd_buffer_cpu: u64,
    /// Bus address of command buffer
    pub cmd_buffer_bus: u64,
    /// CPU pointer to response buffer
    pub resp_buffer_cpu: u64,
    /// Bus address of response buffer
    pub resp_buffer_bus: u64,
}

/// VirtIO-GPU command header.
#[repr(C)]
pub struct GpuCtrlHdr {
    pub cmd_type: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub ring_idx: u8,
    pub padding: [u8; 3],
}

/// Display information from device.
#[repr(C)]
pub struct DisplayInfo {
    /// Display rectangle
    pub rect: GpuRect,
    /// Enabled flag
    pub enabled: u32,
    /// Flags
    pub flags: u32,
}

/// GPU rectangle.
#[repr(C)]
pub struct GpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Resource format.
#[repr(u32)]
pub enum GpuFormat {
    B8G8R8A8Unorm = 1,
    B8G8R8X8Unorm = 2,
    A8R8G8B8Unorm = 3,
    X8R8G8B8Unorm = 4,
    R8G8B8A8Unorm = 67,
    R8G8B8X8Unorm = 68,
}
```

---

# 3. DMA & GPU Resource Management

## 3.1 Design Principles

GPU resource management requires:

1. **Physical address visibility** — Device sees bus addresses, not virtual
2. **Cache coherency** — CPU caches must not hide device writes
3. **Ownership tracking** — Prevent use-while-in-flight bugs
4. **Alignment** — Hardware requires specific alignments
5. **Resource lifecycle** — Create, upload, attach, use, destroy

## 3.2 Memory Layout (4MB GPU Region)

```
Offset      Size        Content                     Notes
────────────────────────────────────────────────────────────────────
0x00000     0x0400      Control Queue Descriptors   64 × 16 bytes
0x00400     0x0088      Control Available Ring      4 + 64×2 + 2 pad
0x00800     0x0208      Control Used Ring           4 + 64×8 + 2 pad
0x01000     0x0200      Cursor Queue Descriptors    32 × 16 bytes
0x01200     0x0048      Cursor Available Ring
0x01400     0x0108      Cursor Used Ring
0x02000     0x1000      Command Buffer              4KB command staging
0x03000     0x1000      Response Buffer             4KB response staging
0x04000     0x10000     Scanout Buffer 0            64KB (1920×1080×4 partial)
0x14000     0x10000     Scanout Buffer 1            64KB (double buffer)
0x24000     0x10000     Scanout Buffer 2            64KB (triple buffer)
0x34000     ...         Resource Pool               Remaining for GPU resources
────────────────────────────────────────────────────────────────────
Minimum:    4MB allocation required
```

## 3.3 Resource Ownership Model

```
                    RESOURCE OWNERSHIP STATE MACHINE
                    
              ┌─────────┐
              │  FREE   │  Not allocated
              └────┬────┘
                   │ create_resource()
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │  Rust code may modify resource
          │   (CPU Access)   │  
          └────────┬─────────┘
                   │ submit_transfer() or submit_command()
                   │ [Ownership transferred to device]
                   ▼
          ┌──────────────────┐
          │   DEVICE-OWNED   │  *** DRIVER MUST NOT ACCESS ***
          │   (GPU Active)   │  Any access is UNDEFINED BEHAVIOR
          └────────┬─────────┘
                   │ fence_signaled()
                   │ [Ownership returned to driver]
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │  Rust code may modify resource
          └────────┬─────────┘
                   │ destroy_resource()
                   ▼
              ┌─────────┐
              │  FREE   │
              └─────────┘

INVARIANT GPU-OWN-1: Each resource is in EXACTLY ONE state at any time.
INVARIANT GPU-OWN-2: Only fence completion may transition DEVICE→DRIVER.
INVARIANT GPU-OWN-3: Accessing DEVICE-OWNED resource is instant UB.
```

## 3.4 Resource Pool Implementation

```rust
// src/resource/pool.rs

/// Pre-allocated resource pool for GPU operations.
pub struct ResourcePool {
    resources: [GpuResource; MAX_RESOURCES],
    free_mask: u64,  // Bitmap of free resources
}

impl ResourcePool {
    /// Allocate a resource. Returns None if pool exhausted.
    pub fn alloc(&mut self) -> Option<&mut GpuResource> {
        if self.free_mask == 0 {
            return None;
        }
        let idx = self.free_mask.trailing_zeros() as usize;
        self.free_mask &= !(1 << idx);
        let resource = &mut self.resources[idx];
        resource.state = ResourceState::DriverOwned;
        Some(resource)
    }

    /// Return resource to pool.
    pub fn free(&mut self, resource: &mut GpuResource) {
        debug_assert_eq!(resource.state, ResourceState::DriverOwned);
        resource.state = ResourceState::Free;
        self.free_mask |= 1 << resource.index;
    }
}
```

---

# 4. VirtIO-GPU Driver Implementation

## 4.1 VirtIO-GPU Overview

VirtIO-GPU is a standard interface for virtual GPU devices in VMs.

**Reference**: VirtIO Specification v1.2

### Key Concepts

| Concept | Description |
|---------|-------------|
| **Control Queue** | Command/response queue (create resource, transfer, flush) |
| **Cursor Queue** | Cursor update queue (optional) |
| **Resource** | GPU-side buffer (can be 2D or 3D texture) |
| **Scanout** | Display output attached to resource |
| **Fence** | Synchronization primitive |

## 4.2 Device Discovery

VirtIO-GPU devices are identified by PCI vendor/device IDs:

```rust
/// VirtIO PCI vendor ID
pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

/// VirtIO-GPU PCI device IDs
pub const VIRTIO_GPU_DEVICE_IDS: &[u16] = &[
    0x1050,  // VirtIO-GPU (transitional)
    0x1040 + 16,  // Modern VirtIO-GPU (device type 16)
];

/// Check if PCI device is VirtIO-GPU.
pub fn is_virtio_gpu(vendor: u16, device: u16) -> bool {
    vendor == VIRTIO_VENDOR_ID && VIRTIO_GPU_DEVICE_IDS.contains(&device)
}
```

## 4.3 Feature Negotiation

```rust
/// VirtIO-GPU feature bits
pub mod features {
    /// VirtIO 1.0+ (modern device)
    pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
    
    /// 3D virgl support
    pub const VIRTIO_GPU_F_VIRGL: u64 = 1 << 0;
    
    /// EDID support
    pub const VIRTIO_GPU_F_EDID: u64 = 1 << 1;
    
    /// Resource UUID support
    pub const VIRTIO_GPU_F_RESOURCE_UUID: u64 = 1 << 2;
    
    /// Blob resource support
    pub const VIRTIO_GPU_F_RESOURCE_BLOB: u64 = 1 << 3;
    
    /// Context init support
    pub const VIRTIO_GPU_F_CONTEXT_INIT: u64 = 1 << 4;
}

/// Required features (device must support, else reject)
pub const REQUIRED_FEATURES: u64 = features::VIRTIO_F_VERSION_1;

/// Desired features (use if available)
pub const DESIRED_FEATURES: u64 = 
    features::VIRTIO_GPU_F_VIRGL;  // 3D acceleration

/// Negotiate features with device.
pub fn negotiate_features(device_features: u64) -> Result<u64, FeatureError> {
    if device_features & REQUIRED_FEATURES != REQUIRED_FEATURES {
        return Err(FeatureError::MissingRequired(REQUIRED_FEATURES));
    }
    
    let our_features = REQUIRED_FEATURES | (DESIRED_FEATURES & device_features);
    Ok(our_features)
}
```

## 4.4 Initialization Sequence

```rust
/// Initialize VirtIO-GPU device.
/// 
/// # Arguments
/// - `mmio_base`: MMIO base address from PCI BAR
/// - `dma`: Pre-allocated DMA region (4MB minimum)
/// 
/// # Returns
/// Initialized driver or error.
pub fn virtio_gpu_init(
    mmio_base: u64,
    dma: &mut DmaRegion,
) -> Result<VirtioGpuDriver, InitError> {
    
    // ═══════════════════════════════════════════════════════════
    // STEP 1: RESET DEVICE
    // ═══════════════════════════════════════════════════════════
    unsafe { asm_gpu_set_status(mmio_base, 0) };
    
    // Wait for reset (bounded)
    let start = unsafe { asm_tsc_read() };
    let timeout = tsc_freq / 10;  // 100ms
    loop {
        let status = unsafe { asm_gpu_get_status(mmio_base) };
        if status == 0 {
            break;
        }
        if unsafe { asm_tsc_read() }.wrapping_sub(start) > timeout {
            return Err(InitError::ResetTimeout);
        }
        for _ in 0..1000 { core::hint::spin_loop(); }
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 2-6: Standard VirtIO initialization
    // (Same as network: ACKNOWLEDGE → DRIVER → FEATURES → FEATURES_OK)
    // ═══════════════════════════════════════════════════════════
    
    // ... (status progression as in network driver)
    
    // ═══════════════════════════════════════════════════════════
    // STEP 7: CONFIGURE VIRTQUEUES
    // ═══════════════════════════════════════════════════════════
    
    // Control Queue (index 0)
    let ctrl_queue = setup_virtqueue(mmio_base, 0, dma, CTRL_QUEUE_SIZE)?;
    
    // Cursor Queue (index 1)
    let cursor_queue = setup_virtqueue(mmio_base, 1, dma, CURSOR_QUEUE_SIZE)?;
    
    // ═══════════════════════════════════════════════════════════
    // STEP 8: GET DISPLAY INFO
    // ═══════════════════════════════════════════════════════════
    let mut display_info = DisplayInfo::default();
    if unsafe { asm_gpu_get_display_info(mmio_base, &mut display_info) } != 0 {
        return Err(InitError::DisplayInfoFailed);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 9: SET DRIVER_OK
    // ═══════════════════════════════════════════════════════════
    unsafe {
        asm_gpu_set_status(mmio_base,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK)
    };
    
    // ═══════════════════════════════════════════════════════════
    // STEP 10: CHECK FOR 3D CAPABILITY
    // ═══════════════════════════════════════════════════════════
    let has_3d = our_features & features::VIRTIO_GPU_F_VIRGL != 0;
    
    Ok(VirtioGpuDriver {
        mmio_base,
        features: our_features,
        ctrl_queue,
        cursor_queue,
        display_info,
        has_3d,
        resource_pool: ResourcePool::new(dma),
        frame_metrics: FrameMetrics::new(),
    })
}
```

## 4.5 Command Submission (Fire-and-Forget)

```rust
impl VirtioGpuDriver {
    /// Submit a GPU command. Returns immediately (no completion wait).
    /// 
    /// # Returns
    /// - `Ok(fence_id)`: Command queued, fence_id for tracking
    /// - `Err(DisplayError::QueueFull)`: No space, try again after polling
    pub fn submit_command(&mut self, cmd: &GpuCommand) -> Result<u64, DisplayError> {
        // Collect any pending responses first
        self.collect_responses();
        
        // Check queue space
        if !self.ctrl_queue.can_submit() {
            return Err(DisplayError::QueueFull);
        }
        
        // Allocate fence for tracking
        let fence_id = self.next_fence_id;
        self.next_fence_id += 1;
        
        // Submit via ASM (includes barriers)
        let result = unsafe {
            asm_gpu_submit_cmd(&mut self.ctrl_queue, cmd.with_fence(fence_id))
        };
        
        if result != 0 {
            return Err(DisplayError::QueueFull);
        }
        
        // Notify device
        unsafe { asm_gpu_notify(&mut self.ctrl_queue) };
        
        // *** DO NOT WAIT FOR COMPLETION ***
        // Completion collected in main loop
        
        Ok(fence_id)
    }
    
    /// Collect command responses. Call in main loop.
    pub fn collect_responses(&mut self) {
        loop {
            let has_response = unsafe { 
                asm_gpu_poll_response(&mut self.ctrl_queue) 
            };
            if has_response == 0 {
                break;
            }
            // Process response, update fence state
            self.process_response();
        }
    }
}
```

---

# 5. Performance & Frame Pacing

## 5.1 Performance Targets

| Target | Value | Measurement |
|--------|-------|-------------|
| Frame rate | ≥60 FPS | Frames per second |
| Frame budget | ≤16.6 ms | Time per frame |
| Jitter | ≤2 ms | Frame time variance |
| Queue latency | <1 ms | Command submit to execute |
| Fence latency | <2 ms | Submit to signal |

## 5.2 Double/Triple Buffering

```rust
/// Frame buffer management with N-buffering
pub struct FrameBufferChain {
    buffers: [ScanoutBuffer; 3],  // Triple buffering
    front: usize,    // Currently displayed
    back: usize,     // Currently rendering to
    pending: usize,  // Waiting for vsync
    buffer_count: usize,  // 2 or 3
}

impl FrameBufferChain {
    /// Get back buffer for rendering
    pub fn acquire(&mut self) -> &mut ScanoutBuffer {
        &mut self.buffers[self.back]
    }
    
    /// Submit back buffer for display
    pub fn present(&mut self) -> Result<(), DisplayError> {
        // Move back to pending
        self.pending = self.back;
        
        // Find next free buffer for back
        self.back = (self.back + 1) % self.buffer_count;
        if self.back == self.front {
            // Would overwrite front buffer, need to wait
            return Err(DisplayError::NoFreeBuffer);
        }
        
        Ok(())
    }
    
    /// Called when vsync fires (pending becomes front)
    pub fn on_vsync(&mut self) {
        let old_front = self.front;
        self.front = self.pending;
        // Old front is now available as back if needed
    }
}
```

## 5.3 Vsync Strategy

```rust
/// Vsync strategy configuration
#[derive(Clone, Copy)]
pub enum VsyncStrategy {
    /// Immediate present, may tear
    Immediate,
    /// Wait for vsync, double buffer
    DoubleBuffered,
    /// Vsync with triple buffer (reduces latency)
    TripleBuffered,
    /// Adaptive vsync (tear if late)
    Adaptive,
}

impl VsyncStrategy {
    pub fn buffer_count(&self) -> usize {
        match self {
            Self::Immediate | Self::DoubleBuffered => 2,
            Self::TripleBuffered | Self::Adaptive => 3,
        }
    }
}
```

## 5.4 Frame Pacing

```rust
/// Frame pacing to minimize jitter
pub struct FramePacer {
    target_frame_time: u64,  // TSC ticks per frame
    frame_start: u64,
    frame_times: [u64; 16],  // Rolling window
    frame_idx: usize,
}

impl FramePacer {
    /// Start frame timing
    pub fn begin_frame(&mut self) {
        self.frame_start = unsafe { asm_tsc_read() };
    }
    
    /// End frame, calculate timing
    pub fn end_frame(&mut self) -> FrameTiming {
        let now = unsafe { asm_tsc_read() };
        let elapsed = now.wrapping_sub(self.frame_start);
        
        // Store for jitter calculation
        self.frame_times[self.frame_idx] = elapsed;
        self.frame_idx = (self.frame_idx + 1) % 16;
        
        FrameTiming {
            elapsed_ticks: elapsed,
            budget_exceeded: elapsed > self.target_frame_time,
            excess_ticks: elapsed.saturating_sub(self.target_frame_time),
        }
    }
    
    /// Calculate jitter (variance in frame times)
    pub fn calculate_jitter(&self) -> u64 {
        let sum: u64 = self.frame_times.iter().sum();
        let avg = sum / 16;
        // Use saturating arithmetic to prevent overflow with large timing values
        let variance: u64 = self.frame_times.iter()
            .map(|&t| {
                let diff = if t > avg { t - avg } else { avg - t };
                // Saturate on overflow to prevent panic in debug builds
                diff.saturating_mul(diff)
            })
            .fold(0u64, |acc, x| acc.saturating_add(x)) / 16;
        // Return approximate standard deviation in ticks
        // Note: This uses integer sqrt approximation for no_std compatibility
        integer_sqrt(variance)
    }
    
    /// Integer square root (for no_std environments)
    fn integer_sqrt(n: u64) -> u64 {
        if n == 0 { return 0; }
        let mut x = n;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + n / x) / 2;
        }
        x
    }
}
```

## 5.5 Latency Instrumentation

```rust
/// Latency metrics collection
pub struct FrameMetrics {
    /// Queue submit to notify
    submit_latency: Histogram,
    /// Notify to fence signal
    fence_latency: Histogram,
    /// Scanout swap latency
    swap_latency: Histogram,
    /// Total frame time
    frame_time: Histogram,
}

impl FrameMetrics {
    /// Record submit latency
    pub fn record_submit(&mut self, start_tsc: u64, end_tsc: u64) {
        let latency = end_tsc.wrapping_sub(start_tsc);
        self.submit_latency.record(latency);
    }
    
    /// Record fence signal latency
    pub fn record_fence(&mut self, submit_tsc: u64, signal_tsc: u64) {
        let latency = signal_tsc.wrapping_sub(submit_tsc);
        self.fence_latency.record(latency);
    }
    
    /// Get metrics summary
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            submit_p50: self.submit_latency.percentile(50),
            submit_p99: self.submit_latency.percentile(99),
            fence_p50: self.fence_latency.percentile(50),
            fence_p99: self.fence_latency.percentile(99),
            frame_p50: self.frame_time.percentile(50),
            frame_p99: self.frame_time.percentile(99),
        }
    }
}
```

## 5.6 Power/Thermal Considerations

```rust
/// Power-aware submission batching
pub struct CommandBatcher {
    commands: Vec<GpuCommand>,
    max_batch_size: usize,
    batch_timeout_ticks: u64,
    batch_start: u64,
}

impl CommandBatcher {
    /// Add command to batch
    pub fn add(&mut self, cmd: GpuCommand) -> Option<Vec<GpuCommand>> {
        self.commands.push(cmd);
        
        // Flush if batch full
        if self.commands.len() >= self.max_batch_size {
            return Some(self.flush());
        }
        
        // Start batch timer if first command
        if self.commands.len() == 1 {
            self.batch_start = unsafe { asm_tsc_read() };
        }
        
        None
    }
    
    /// Check if batch should flush due to timeout
    pub fn check_timeout(&mut self) -> Option<Vec<GpuCommand>> {
        if self.commands.is_empty() {
            return None;
        }
        
        let now = unsafe { asm_tsc_read() };
        if now.wrapping_sub(self.batch_start) > self.batch_timeout_ticks {
            return Some(self.flush());
        }
        
        None
    }
    
    /// Flush batch
    fn flush(&mut self) -> Vec<GpuCommand> {
        core::mem::take(&mut self.commands)
    }
}
```

---

# 6. State Machines

## 6.1 Design Principles

State machines replace all blocking patterns. Each state machine:

1. **Has a `step()` method** that advances by one logical step
2. **Returns immediately** without waiting for external events
3. **Checks timeouts** as observations, not waits
4. **Transitions on conditions** being met at call time

## 6.2 Display State Machine

```rust
/// Display rendering state machine
pub enum DisplayState {
    /// Initializing display
    Init,
    /// Waiting for display mode set
    SettingMode { start_tsc: u64 },
    /// Creating scanout resources
    CreatingResources { mode: DisplayMode, start_tsc: u64 },
    /// Normal rendering operation
    Rendering { 
        frame_start: u64,
        pending_fence: Option<u64>,
    },
    /// Error state
    Failed { error: DisplayError },
}

impl DisplayState {
    pub fn step(
        &mut self,
        driver: &mut impl DisplayDriver,
        now_tsc: u64,
        timeouts: &TimeoutConfig,
    ) -> StepResult {
        match self {
            Self::Init => {
                *self = Self::SettingMode { start_tsc: now_tsc };
                StepResult::Pending
            }
            
            Self::SettingMode { start_tsc } => {
                // Check timeout
                if now_tsc.wrapping_sub(*start_tsc) > timeouts.mode_set() {
                    *self = Self::Failed { error: DisplayError::Timeout };
                    return StepResult::Failed;
                }
                
                // Try to set mode
                match driver.set_mode(1920, 1080, PixelFormat::BGRA8888) {
                    Ok(()) => {
                        *self = Self::CreatingResources { 
                            mode: DisplayMode::new(1920, 1080),
                            start_tsc: now_tsc,
                        };
                        StepResult::Pending
                    }
                    Err(DisplayError::QueueFull) => StepResult::Pending,
                    Err(e) => {
                        *self = Self::Failed { error: e };
                        StepResult::Failed
                    }
                }
            }
            
            Self::CreatingResources { mode, start_tsc } => {
                // Check timeout
                if now_tsc.wrapping_sub(*start_tsc) > timeouts.resource_create() {
                    *self = Self::Failed { error: DisplayError::Timeout };
                    return StepResult::Failed;
                }
                
                // Create scanout resources
                match driver.create_scanout(0, mode.width, mode.height) {
                    Ok(_) => {
                        *self = Self::Rendering { 
                            frame_start: now_tsc,
                            pending_fence: None,
                        };
                        StepResult::Pending
                    }
                    Err(DisplayError::QueueFull) => StepResult::Pending,
                    Err(e) => {
                        *self = Self::Failed { error: e };
                        StepResult::Failed
                    }
                }
            }
            
            Self::Rendering { frame_start, pending_fence } => {
                // Check pending fence
                if let Some(fence_id) = *pending_fence {
                    match driver.poll_fence(fence_id) {
                        Ok(true) => {
                            // Fence signaled, can start next frame
                            *pending_fence = None;
                        }
                        Ok(false) => {
                            // Still waiting
                            return StepResult::Pending;
                        }
                        Err(e) => {
                            *self = Self::Failed { error: e };
                            return StepResult::Failed;
                        }
                    }
                }
                
                StepResult::Pending
            }
            
            Self::Failed { .. } => StepResult::Failed,
        }
    }
}
```

---

# 7. Main Loop & Execution Model

## 7.1 Display Main Loop Integration

```rust
/// Main loop with display integration
pub fn main_loop(
    net_device: &mut impl NetworkDevice,
    display_device: &mut impl DisplayDriver,
    handoff: &BootHandoff,
) -> ! {
    let timeouts = TimeoutConfig::new(handoff.tsc_freq);
    let mut frame_pacer = FramePacer::new(handoff.tsc_freq, 60);  // 60 FPS target
    
    loop {
        let iteration_start = unsafe { asm_tsc_read() };
        
        // ═══════════════════════════════════════════════════════
        // PHASE 1: POLL GPU FENCES
        // Budget: ~50µs
        // ═══════════════════════════════════════════════════════
        display_device.collect_responses();
        
        // ═══════════════════════════════════════════════════════
        // PHASE 2: NETWORK PROCESSING
        // Budget: ~500µs
        // ═══════════════════════════════════════════════════════
        net_device.refill_rx_queue();
        // ... network poll ...
        
        // ═══════════════════════════════════════════════════════
        // PHASE 3: RENDER FRAME
        // Budget: ~10ms
        // ═══════════════════════════════════════════════════════
        frame_pacer.begin_frame();
        
        // Application rendering
        let fb = display_device.acquire_frame_buffer()?;
        render_scene(&mut fb);
        
        // Upload and present
        display_device.upload_resource(fb)?;
        display_device.flush()?;
        
        let timing = frame_pacer.end_frame();
        
        // ═══════════════════════════════════════════════════════
        // PHASE 4: COLLECT COMPLETIONS
        // Budget: ~50µs
        // ═══════════════════════════════════════════════════════
        net_device.collect_tx_completions();
        display_device.collect_responses();
        
        // ═══════════════════════════════════════════════════════
        // FRAME TIMING CHECK
        // ═══════════════════════════════════════════════════════
        if timing.budget_exceeded {
            // Log frame budget overrun
        }
    }
}
```

---

# 8. Boot Integration

## 8.1 BootHandoff Display Fields

```rust
/// Data passed from UEFI boot phase to bare-metal phase.
pub struct BootHandoff {
    // ... existing fields ...
    
    // ═══════════════════════════════════════════════════════════
    // GPU INFORMATION
    // ═══════════════════════════════════════════════════════════
    
    /// GPU MMIO base address (from PCI BAR)
    pub gpu_mmio_base: u64,
    
    /// GPU PCI location
    pub gpu_pci_bus: u8,
    pub gpu_pci_device: u8,
    pub gpu_pci_function: u8,
    
    /// GPU type: 0=None, 1=VirtIO-GPU, 2=Intel, 3=AMD, 4=NVIDIA
    pub gpu_type: u8,
    
    /// GPU features (from capability probe)
    pub gpu_features: u64,
    
    // ═══════════════════════════════════════════════════════════
    // GPU DMA REGION (separate from network)
    // ═══════════════════════════════════════════════════════════
    
    /// GPU DMA CPU pointer
    pub gpu_dma_cpu_ptr: u64,
    
    /// GPU DMA bus address
    pub gpu_dma_bus_addr: u64,
    
    /// GPU DMA size (minimum 4MB)
    pub gpu_dma_size: u64,
    
    // ═══════════════════════════════════════════════════════════
    // UEFI GOP FALLBACK (for compatibility)
    // ═══════════════════════════════════════════════════════════
    
    /// Framebuffer base from UEFI GOP
    pub uefi_fb_base: u64,
    
    /// Framebuffer size
    pub uefi_fb_size: u64,
    
    /// Framebuffer dimensions
    pub uefi_fb_width: u32,
    pub uefi_fb_height: u32,
    pub uefi_fb_stride: u32,
    pub uefi_fb_format: u32,
}
```

## 8.2 Pre-EBS GPU Discovery

```rust
/// Scan PCI for GPU device.
pub fn find_gpu_device(pci_io: &PciRootBridgeIoProtocol) -> Option<GpuInfo> {
    // Priority order: Vendor GPU → VirtIO-GPU → Simple framebuffer
    
    // Check for vendor GPUs first (future)
    // if let Some(info) = find_amd_gpu(pci_io) { return Some(info); }
    // if let Some(info) = find_intel_gpu(pci_io) { return Some(info); }
    
    // Check for VirtIO-GPU
    for bus in 0..=255 {
        for device in 0..32 {
            for function in 0..8 {
                let vendor_device = pci_read_config32(pci_io, bus, device, function, 0);
                let vendor = (vendor_device & 0xFFFF) as u16;
                let device_id = ((vendor_device >> 16) & 0xFFFF) as u16;
                
                if is_virtio_gpu(vendor, device_id) {
                    let bar0 = pci_read_config32(pci_io, bus, device, function, 0x10);
                    let mmio_base = (bar0 & 0xFFFFFFF0) as u64;
                    
                    return Some(GpuInfo {
                        mmio_base,
                        pci_bus: bus,
                        pci_device: device,
                        pci_function: function,
                        gpu_type: GpuType::VirtioGpu,
                    });
                }
            }
        }
    }
    
    // Fallback to UEFI GOP framebuffer (always available)
    None
}
```

---

# 9. Driver Abstraction Layer

## 9.1 DisplayDriver Trait

```rust
/// Core display driver interface.
pub trait DisplayDriver {
    /// Get driver capabilities.
    fn capabilities(&self) -> &DisplayCapabilities;
    
    /// Set display mode.
    fn set_mode(&mut self, width: u32, height: u32, format: PixelFormat) -> Result<(), DisplayError>;
    
    /// Create GPU resource.
    fn create_resource(&mut self, width: u32, height: u32, format: PixelFormat) -> Result<ResourceId, DisplayError>;
    
    /// Upload data to resource.
    fn upload_resource(&mut self, id: ResourceId, data: &[u8]) -> Result<(), DisplayError>;
    
    /// Set scanout to resource.
    fn set_scanout(&mut self, scanout_id: u32, resource: ResourceId) -> Result<(), DisplayError>;
    
    /// Flush pending operations.
    fn flush(&mut self) -> Result<(), DisplayError>;
    
    /// Create synchronization fence.
    fn create_fence(&mut self) -> Result<FenceId, DisplayError>;
    
    /// Poll fence status (non-blocking).
    fn poll_fence(&mut self, id: FenceId) -> Result<bool, DisplayError>;
    
    /// Collect command responses.
    fn collect_responses(&mut self);
    
    /// Teardown driver.
    fn teardown(&mut self);
}

/// Display capabilities.
pub struct DisplayCapabilities {
    /// Maximum supported width
    pub max_width: u32,
    /// Maximum supported height
    pub max_height: u32,
    /// Supported pixel formats
    pub formats: &'static [PixelFormat],
    /// 3D acceleration available
    pub has_3d: bool,
    /// Number of scanouts
    pub num_scanouts: u32,
    /// Maximum resources
    pub max_resources: u32,
}
```

## 9.2 Adding a New GPU Driver

To add support for a new GPU (e.g., Intel):

### Step 1: Create ASM Functions

```nasm
; asm/intel_gpu.s
global asm_intel_gpu_init
global asm_intel_gpu_flip
; ... etc
```

### Step 2: Create Driver Module

```rust
// src/device/intel_gpu.rs

pub struct IntelGpuDriver {
    mmio_base: u64,
    // ... driver state
}

impl DisplayDriver for IntelGpuDriver {
    // Implement all trait methods
}
```

### Step 3: Update Factory

```rust
pub enum UnifiedDisplayDevice {
    VirtioGpu(VirtioGpuDriver),
    Intel(IntelGpuDriver),
    Framebuffer(FramebufferDriver),
}

impl DisplayDriver for UnifiedDisplayDevice {
    // Delegate to inner driver
}
```

---

# 10. Testing & Validation

## 10.1 QEMU Test Configuration

```bash
#!/bin/bash
# Run MorpheusX with VirtIO-GPU

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

## 10.2 Performance Tests

### Test: 60 FPS Sustained

```
1. Boot MorpheusX with VirtIO-GPU
2. Enter rendering loop
3. Measure frame times for 1000 frames
4. Assert: 99% of frames < 16.6ms
5. Assert: Average FPS ≥ 60
```

### Test: Jitter < 2ms

```
1. Measure frame time variance
2. Calculate standard deviation
3. Assert: σ < 2ms
```

### Test: Fence Latency

```
1. Create fence
2. Submit command with fence
3. Measure time to fence signal
4. Assert: P99 latency < 2ms
```

## 10.3 Invariant Verification

| Category | Invariants | Verification Method |
|----------|------------|---------------------|
| ASM | Barrier correctness | Code review, hardware test |
| Resource | Ownership tracking | Debug assertions |
| Frame | Budget compliance | Runtime instrumentation |
| Queue | No blocking | Code review |

---

*End of Implementation Guide*
