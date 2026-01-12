# MorpheusX Hardware Extension Guide

**Version**: 1.0  
**Status**: AUTHORITATIVE  
**Date**: January 2026  

---

## Overview

This document describes the architecture for extending MorpheusX display stack with drop-in hardware drivers for real GPUs (AMD, Intel, NVIDIA, and others). The design enables adding new GPU support without modifying the core display stack.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Driver Capability Interface](#2-driver-capability-interface)
3. [ASM Hardware Layer](#3-asm-hardware-layer)
4. [Rust↔ASM Boundary](#4-rustasm-boundary)
5. [Driver Discovery & Registration](#5-driver-discovery--registration)
6. [Capability Negotiation](#6-capability-negotiation)
7. [Fallback Ordering](#7-fallback-ordering)
8. [Implementation Guide](#8-implementation-guide)
9. [Memory Management](#9-memory-management)

---

# 1. Architecture Overview

## 1.1 Drop-in Driver Architecture

The hardware extension architecture enables new GPU support via modular drivers that implement a common interface:

```
┌─────────────────────────────────────────────────────────────────┐
│                     APPLICATION LAYER                           │
│              (Uses DisplayDriver trait)                         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     DISPLAY API                                 │
│           (DisplayDriver trait, UnifiedDisplayDevice)           │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌──────────────────┐  ┌──────────────┐  ┌──────────────────────┐
│   VirtIO-GPU     │  │  AMD GPU     │  │   Intel/NVIDIA/...   │
│   (baseline)     │  │  (vendor)    │  │   (future drop-ins)  │
└──────────────────┘  └──────────────┘  └──────────────────────┘
          │                   │                   │
          ▼                   ▼                   ▼
┌─────────────────────────────────────────────────────────────────┐
│                     ASM HARDWARE LAYER                          │
│    Common interface: MMIO, PIO, DMA, Interrupts, Barriers       │
└─────────────────────────────────────────────────────────────────┘
          │                   │                   │
          ▼                   ▼                   ▼
┌──────────────────┐  ┌──────────────┐  ┌──────────────────────┐
│   VirtIO-GPU HW  │  │  AMD RDNA HW │  │   Intel/NVIDIA HW    │
└──────────────────┘  └──────────────┘  └──────────────────────┘
```

## 1.2 Key Design Principles

1. **Stable Interface**: `DisplayDriver` trait is frozen; drivers implement it
2. **ASM Isolation**: Hardware-touching code in ASM, not inline assembly
3. **Common Capability Set**: All drivers expose same minimal capability interface
4. **Graceful Degradation**: Fallback chain: Vendor GPU → VirtIO-GPU → Simple Framebuffer

---

# 2. Driver Capability Interface

## 2.1 Minimal Capability Interface

Every GPU driver MUST implement this minimal interface:

```rust
/// Minimal GPU capability interface.
/// All drivers implement this.
pub trait GpuCapability {
    /// Mode setting: set display resolution and format.
    fn mode_set(
        &mut self, 
        width: u32, 
        height: u32, 
        format: PixelFormat
    ) -> Result<(), DisplayError>;
    
    /// Scanout attach: attach framebuffer to display output.
    fn scanout_attach(
        &mut self,
        scanout_id: u32,
        resource: ResourceId,
        rect: Rect,
    ) -> Result<(), DisplayError>;
    
    /// Resource upload: transfer pixel data to GPU.
    fn resource_upload(
        &mut self,
        resource: ResourceId,
        data: &[u8],
        rect: Rect,
    ) -> Result<(), DisplayError>;
    
    /// Fence create and wait: synchronization primitive.
    fn fence_create(&mut self) -> Result<FenceId, DisplayError>;
    fn fence_wait(&mut self, fence: FenceId) -> Result<bool, DisplayError>;
    
    /// Teardown: release all resources and shutdown.
    fn teardown(&mut self);
}
```

## 2.2 Extended Capabilities

Drivers MAY implement additional capabilities:

```rust
/// 3D acceleration capability.
pub trait Gpu3DCapability: GpuCapability {
    fn create_3d_context(&mut self) -> Result<ContextId, DisplayError>;
    fn submit_3d_commands(&mut self, ctx: ContextId, cmds: &[u8]) -> Result<FenceId, DisplayError>;
    fn destroy_3d_context(&mut self, ctx: ContextId) -> Result<(), DisplayError>;
}

/// Video decode capability.
pub trait GpuVideoCapability: GpuCapability {
    fn decode_begin(&mut self, codec: VideoCodec) -> Result<DecoderHandle, DisplayError>;
    fn decode_frame(&mut self, handle: DecoderHandle, data: &[u8]) -> Result<ResourceId, DisplayError>;
    fn decode_end(&mut self, handle: DecoderHandle) -> Result<(), DisplayError>;
}

/// Cursor capability.
pub trait GpuCursorCapability: GpuCapability {
    fn cursor_set(&mut self, resource: ResourceId, hot_x: u32, hot_y: u32) -> Result<(), DisplayError>;
    fn cursor_move(&mut self, x: u32, y: u32) -> Result<(), DisplayError>;
    fn cursor_hide(&mut self) -> Result<(), DisplayError>;
}
```

## 2.3 Capability Flags

```rust
bitflags! {
    /// GPU capability flags.
    pub struct GpuCapFlags: u64 {
        /// Basic 2D operations (always required)
        const CAP_2D            = 1 << 0;
        /// 3D acceleration (virgl, native)
        const CAP_3D            = 1 << 1;
        /// Video decode
        const CAP_VIDEO_DECODE  = 1 << 2;
        /// Video encode
        const CAP_VIDEO_ENCODE  = 1 << 3;
        /// Hardware cursor
        const CAP_CURSOR        = 1 << 4;
        /// Multi-head output
        const CAP_MULTIHEAD     = 1 << 5;
        /// EDID support
        const CAP_EDID          = 1 << 6;
        /// HDR support
        const CAP_HDR           = 1 << 7;
        /// Compute shaders
        const CAP_COMPUTE       = 1 << 8;
        /// Raytracing
        const CAP_RAYTRACING    = 1 << 9;
    }
}
```

---

# 3. ASM Hardware Layer

## 3.1 Common ASM Interface

Every GPU driver uses a common ASM interface for hardware access:

### 3.1.1 PCI Enumeration and BAR Mapping

```asm
; ═══════════════════════════════════════════════════════════════
; asm_pci_enum_class
; ═══════════════════════════════════════════════════════════════
; Enumerate PCI devices by class code.
;
; Parameters:
;   RCX = class_code: u32 (e.g., 0x030000 for VGA)
;   RDX = *mut PciDeviceList (output buffer)
;   R8  = max_devices: u32
; Returns:
;   RAX = number of devices found
; ═══════════════════════════════════════════════════════════════
global asm_pci_enum_class

; ═══════════════════════════════════════════════════════════════
; asm_pci_read_bar
; ═══════════════════════════════════════════════════════════════
; Read PCI BAR value.
;
; Parameters:
;   RCX = bus: u8
;   RDX = device: u8
;   R8  = function: u8
;   R9  = bar_index: u8 (0-5)
; Returns:
;   RAX = BAR value (may need 64-bit for large BARs)
; ═══════════════════════════════════════════════════════════════
global asm_pci_read_bar

; ═══════════════════════════════════════════════════════════════
; asm_pci_write_bar
; ═══════════════════════════════════════════════════════════════
; Write PCI BAR value (for sizing).
;
; Parameters:
;   RCX = bus: u8
;   RDX = device: u8
;   R8  = function: u8
;   R9  = bar_index: u8
;   [RSP+0x28] = value: u64
; Returns: None
; ═══════════════════════════════════════════════════════════════
global asm_pci_write_bar

; ═══════════════════════════════════════════════════════════════
; asm_pci_enable_mmio
; ═══════════════════════════════════════════════════════════════
; Enable MMIO and bus mastering for device.
;
; Parameters:
;   RCX = bus: u8
;   RDX = device: u8
;   R8  = function: u8
; Returns:
;   RAX = 0 success, 1 error
; ═══════════════════════════════════════════════════════════════
global asm_pci_enable_mmio
```

### 3.1.2 MMIO/PIO Register Access

```asm
; ═══════════════════════════════════════════════════════════════
; asm_mmio_read32 / asm_mmio_write32
; ═══════════════════════════════════════════════════════════════
; 32-bit MMIO read/write (already defined in generic ASM)
global asm_mmio_read32
global asm_mmio_write32

; ═══════════════════════════════════════════════════════════════
; asm_mmio_read64 / asm_mmio_write64
; ═══════════════════════════════════════════════════════════════
; 64-bit MMIO read/write
global asm_mmio_read64
global asm_mmio_write64

; ═══════════════════════════════════════════════════════════════
; asm_pio_read8 / asm_pio_write8
; ═══════════════════════════════════════════════════════════════
; Port I/O operations (for legacy devices)
;
; Parameters:
;   RCX = port: u16
;   RDX = value: u8 (for write)
; Returns:
;   RAX = value: u8 (for read)
; ═══════════════════════════════════════════════════════════════
global asm_pio_read8
global asm_pio_write8
global asm_pio_read16
global asm_pio_write16
global asm_pio_read32
global asm_pio_write32
```

### 3.1.3 DMA/IOMMU Operations

```asm
; ═══════════════════════════════════════════════════════════════
; asm_dma_alloc
; ═══════════════════════════════════════════════════════════════
; Allocate DMA-capable memory (identity-mapped).
;
; Parameters:
;   RCX = size: u64
;   RDX = alignment: u64
;   R8  = flags: u32 (0=normal, 1=below4G)
; Returns:
;   RAX = physical address (0 on failure)
;   RDX = cpu_ptr (same as physical for identity map)
; ═══════════════════════════════════════════════════════════════
global asm_dma_alloc

; ═══════════════════════════════════════════════════════════════
; asm_dma_map
; ═══════════════════════════════════════════════════════════════
; Map buffer for DMA (returns bus address, handles IOMMU).
;
; Parameters:
;   RCX = cpu_ptr: u64
;   RDX = size: u64
;   R8  = direction: u32 (0=TO_DEVICE, 1=FROM_DEVICE, 2=BIDIRECTIONAL)
; Returns:
;   RAX = bus_addr (for device descriptors)
; ═══════════════════════════════════════════════════════════════
global asm_dma_map

; ═══════════════════════════════════════════════════════════════
; asm_dma_sync
; ═══════════════════════════════════════════════════════════════
; Synchronize DMA buffer (cache operations if needed).
;
; Parameters:
;   RCX = cpu_ptr: u64
;   RDX = size: u64
;   R8  = direction: u32
; Returns: None
; ═══════════════════════════════════════════════════════════════
global asm_dma_sync
```

### 3.1.4 Interrupts (MSI/MSI-X, Legacy INTx)

```asm
; ═══════════════════════════════════════════════════════════════
; asm_msi_enable
; ═══════════════════════════════════════════════════════════════
; Enable MSI for device.
;
; Parameters:
;   RCX = bus: u8
;   RDX = device: u8
;   R8  = function: u8
;   R9  = vector: u8 (IDT vector)
; Returns:
;   RAX = 0 success, 1 MSI not supported, 2 error
; ═══════════════════════════════════════════════════════════════
global asm_msi_enable

; ═══════════════════════════════════════════════════════════════
; asm_msix_enable
; ═══════════════════════════════════════════════════════════════
; Enable MSI-X for device.
;
; Parameters:
;   RCX = bus: u8
;   RDX = device: u8
;   R8  = function: u8
;   R9  = *MsixConfig (array of vector configurations)
; Returns:
;   RAX = 0 success, 1 MSI-X not supported, 2 error
; ═══════════════════════════════════════════════════════════════
global asm_msix_enable

; ═══════════════════════════════════════════════════════════════
; asm_poll_interrupt
; ═══════════════════════════════════════════════════════════════
; Poll for interrupt status (for polling mode).
;
; Parameters:
;   RCX = isr_addr: u64 (interrupt status register)
; Returns:
;   RAX = interrupt status bits
; ═══════════════════════════════════════════════════════════════
global asm_poll_interrupt
```

### 3.1.5 Cache/TLB Barriers and Memory Ordering

```asm
; ═══════════════════════════════════════════════════════════════
; Memory barriers (already defined)
; ═══════════════════════════════════════════════════════════════
global asm_bar_sfence   ; Store fence
global asm_bar_lfence   ; Load fence
global asm_bar_mfence   ; Full fence

; ═══════════════════════════════════════════════════════════════
; asm_clflush
; ═══════════════════════════════════════════════════════════════
; Flush cache line.
;
; Parameters:
;   RCX = address: u64 (cache-line aligned)
; Returns: None
; ═══════════════════════════════════════════════════════════════
global asm_clflush

; ═══════════════════════════════════════════════════════════════
; asm_clflushopt
; ═══════════════════════════════════════════════════════════════
; Optimized cache line flush (if supported).
;
; Parameters:
;   RCX = address: u64
; Returns: None
; ═══════════════════════════════════════════════════════════════
global asm_clflushopt

; ═══════════════════════════════════════════════════════════════
; asm_wbinvd
; ═══════════════════════════════════════════════════════════════
; Write-back and invalidate all caches (expensive!).
;
; Parameters: None
; Returns: None
; Note: Use sparingly, only at critical initialization
; ═══════════════════════════════════════════════════════════════
global asm_wbinvd

; ═══════════════════════════════════════════════════════════════
; asm_invlpg
; ═══════════════════════════════════════════════════════════════
; Invalidate TLB entry.
;
; Parameters:
;   RCX = virtual_address: u64
; Returns: None
; ═══════════════════════════════════════════════════════════════
global asm_invlpg
```

---

# 4. Rust↔ASM Boundary

## 4.1 Stable ABI Contract

All Rust↔ASM calls use Microsoft x64 ABI as defined in DISPLAY_ASM_RUST_ABI_CONTRACT.md.

## 4.2 Driver-Specific ASM

Each vendor driver implements device-specific ASM functions:

### 4.2.1 VirtIO-GPU (Baseline)

```rust
// Already defined in DISPLAY_ASM_RUST_ABI_CONTRACT.md
extern "win64" {
    fn asm_gpu_submit_cmd(queue: *mut CtrlQueueState, cmd: *const GpuCommand) -> u32;
    fn asm_gpu_poll_response(queue: *mut CtrlQueueState) -> u32;
    // ... etc
}
```

### 4.2.2 AMD GPU (Example)

```rust
/// AMD GPU-specific ASM bindings
extern "win64" {
    /// Initialize AMD GPU from MMIO base
    fn asm_amd_gpu_init(mmio_base: u64, doorbell_base: u64) -> u32;
    
    /// Submit command to graphics ring
    fn asm_amd_submit_gfx(ring: *mut AmdRingState, cmd: *const u32, dwords: u32) -> u32;
    
    /// Submit command to SDMA ring
    fn asm_amd_submit_sdma(ring: *mut AmdRingState, cmd: *const u32, dwords: u32) -> u32;
    
    /// Ring doorbell
    fn asm_amd_ring_doorbell(doorbell_base: u64, ring_id: u32, value: u32);
    
    /// Poll fence
    fn asm_amd_poll_fence(fence_addr: u64, expected: u64) -> u32;
    
    /// Set display mode via DCE/DCN
    fn asm_amd_set_mode(mmio: u64, crtc: u32, width: u32, height: u32) -> u32;
    
    /// Program surface
    fn asm_amd_program_surface(mmio: u64, surface: *const AmdSurface) -> u32;
    
    /// Flip to surface
    fn asm_amd_flip(mmio: u64, crtc: u32, surface_addr: u64) -> u32;
}
```

### 4.2.3 Intel GPU (Example)

```rust
/// Intel GPU-specific ASM bindings
extern "win64" {
    /// Initialize Intel GPU
    fn asm_intel_gpu_init(mmio_base: u64, gtt_base: u64) -> u32;
    
    /// Allocate GTT entries
    fn asm_intel_gtt_insert(gtt: u64, index: u32, phys_addr: u64, flags: u32);
    
    /// Submit to render ring
    fn asm_intel_submit_render(ring: *mut IntelRingState, cmd: *const u32, dwords: u32) -> u32;
    
    /// Submit to BLT ring
    fn asm_intel_submit_blt(ring: *mut IntelRingState, cmd: *const u32, dwords: u32) -> u32;
    
    /// Program plane
    fn asm_intel_program_plane(mmio: u64, pipe: u32, plane: *const IntelPlane) -> u32;
    
    /// Flip plane
    fn asm_intel_flip_plane(mmio: u64, pipe: u32, surface_addr: u64) -> u32;
}
```

---

# 5. Driver Discovery & Registration

## 5.1 PCI ID Registration

Each driver registers PCI IDs it supports:

```rust
/// Driver registration entry
pub struct DriverRegistration {
    /// Driver name
    pub name: &'static str,
    /// PCI vendor ID (or 0 for any)
    pub vendor_id: u16,
    /// PCI device IDs supported
    pub device_ids: &'static [u16],
    /// Class code (e.g., 0x030000 for VGA)
    pub class_code: u32,
    /// Driver factory function
    pub create: fn(PciDevice, &mut DmaRegion) -> Result<Box<dyn DisplayDriver>, InitError>,
    /// Priority (higher = preferred)
    pub priority: u32,
}

/// Global driver registry
static DRIVER_REGISTRY: &[DriverRegistration] = &[
    // AMD RDNA2 GPUs
    DriverRegistration {
        name: "amd_rdna2",
        vendor_id: 0x1002,  // AMD
        device_ids: &[0x73BF, 0x73A5, 0x73DF],  // RX 6800, etc.
        class_code: 0x030000,
        create: amd_rdna2_create,
        priority: 100,
    },
    // Intel Xe GPUs
    DriverRegistration {
        name: "intel_xe",
        vendor_id: 0x8086,  // Intel
        device_ids: &[0x56A0, 0x56A1],  // Arc A770, etc.
        class_code: 0x030000,
        create: intel_xe_create,
        priority: 100,
    },
    // VirtIO-GPU (baseline, always available)
    DriverRegistration {
        name: "virtio_gpu",
        vendor_id: 0x1AF4,  // Red Hat
        device_ids: &[0x1050, 0x1050 + 16],  // VirtIO-GPU
        class_code: 0x030000,
        create: virtio_gpu_create,
        priority: 50,
    },
    // Simple framebuffer (lowest priority fallback)
    DriverRegistration {
        name: "simple_fb",
        vendor_id: 0,  // Any
        device_ids: &[],
        class_code: 0x030000,
        create: simple_fb_create,
        priority: 1,
    },
];
```

## 5.2 Discovery Process

```rust
/// Find best GPU driver for system
pub fn discover_gpu_driver(
    dma: &mut DmaRegion,
) -> Result<Box<dyn DisplayDriver>, InitError> {
    // Enumerate PCI display devices
    let devices = enumerate_pci_class(0x030000)?;
    
    // Try each device with matching driver
    let mut best_match: Option<(u32, Box<dyn DisplayDriver>)> = None;
    
    for device in devices {
        for reg in DRIVER_REGISTRY {
            if matches_registration(&device, reg) {
                match (reg.create)(device, dma) {
                    Ok(driver) => {
                        if best_match.as_ref().map(|(p, _)| reg.priority > *p).unwrap_or(true) {
                            best_match = Some((reg.priority, driver));
                        }
                    }
                    Err(e) => {
                        log::warn!("Driver {} failed for device {:04x}:{:04x}: {:?}",
                                  reg.name, device.vendor_id, device.device_id, e);
                    }
                }
            }
        }
    }
    
    best_match
        .map(|(_, driver)| driver)
        .ok_or(InitError::NoDriverFound)
}
```

---

# 6. Capability Negotiation

## 6.1 Probe and Negotiate

```rust
impl<D: DisplayDriver> UnifiedDisplayDevice<D> {
    /// Probe device capabilities
    pub fn probe_capabilities(&self) -> GpuCapFlags {
        let mut flags = GpuCapFlags::CAP_2D;  // Always have 2D
        
        if self.driver.has_3d() {
            flags |= GpuCapFlags::CAP_3D;
        }
        if self.driver.has_cursor() {
            flags |= GpuCapFlags::CAP_CURSOR;
        }
        if self.driver.has_video_decode() {
            flags |= GpuCapFlags::CAP_VIDEO_DECODE;
        }
        // ... etc
        
        flags
    }
    
    /// Negotiate required capabilities
    pub fn require_capabilities(&self, required: GpuCapFlags) -> Result<(), InitError> {
        let available = self.probe_capabilities();
        let missing = required - available;
        
        if !missing.is_empty() {
            return Err(InitError::MissingCapabilities(missing));
        }
        
        Ok(())
    }
}
```

## 6.2 Feature Detection

```rust
/// Detect GPU features via capability probing
pub fn detect_features(driver: &dyn DisplayDriver) -> GpuFeatures {
    GpuFeatures {
        max_width: driver.max_resolution().0,
        max_height: driver.max_resolution().1,
        max_surfaces: driver.max_surfaces(),
        has_3d: driver.has_3d(),
        has_compute: driver.has_compute(),
        vram_size: driver.vram_size(),
        formats: driver.supported_formats(),
    }
}
```

---

# 7. Fallback Ordering

## 7.1 Default Fallback Chain

```
                    ┌────────────────┐
                    │ Probe PCI GPUs │
                    └───────┬────────┘
                            │
            ┌───────────────┼───────────────┐
            ▼               ▼               ▼
    ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
    │  AMD GPU     │ │ Intel GPU    │ │ NVIDIA GPU   │
    │ (priority:   │ │ (priority:   │ │ (priority:   │
    │   100)       │ │   100)       │ │   100)       │
    └──────┬───────┘ └──────┬───────┘ └──────┬───────┘
           │                │                │
           │  Init failed?  │  Init failed?  │  Init failed?
           ▼                ▼                ▼
        ┌───────────────────────────────────────┐
        │             VirtIO-GPU                │
        │           (priority: 50)              │
        └───────────────────┬───────────────────┘
                            │
                            │  Init failed?
                            ▼
        ┌───────────────────────────────────────┐
        │         Simple Framebuffer            │
        │           (priority: 1)               │
        │  (Uses UEFI GOP framebuffer info)     │
        └───────────────────────────────────────┘
```

## 7.2 Fallback Implementation

```rust
/// Try to create display driver with fallback
pub fn create_display_driver_with_fallback(
    handoff: &BootHandoff,
    dma: &mut DmaRegion,
) -> Box<dyn DisplayDriver> {
    // Try hardware discovery first
    if let Ok(driver) = discover_gpu_driver(dma) {
        return driver;
    }
    
    // Try VirtIO-GPU explicitly
    if handoff.gpu_type == GPU_TYPE_VIRTIO && handoff.gpu_mmio_base != 0 {
        if let Ok(driver) = VirtioGpuDriver::new(handoff.gpu_mmio_base, dma) {
            return Box::new(driver);
        }
    }
    
    // Fallback to simple framebuffer
    if handoff.uefi_fb_base != 0 {
        let fb = SimpleFramebuffer::new(
            handoff.uefi_fb_base,
            handoff.uefi_fb_width,
            handoff.uefi_fb_height,
            handoff.uefi_fb_stride,
            handoff.uefi_fb_format.into(),
        );
        return Box::new(fb);
    }
    
    // No display available
    panic!("No display driver available");
}
```

---

# 8. Implementation Guide

## 8.1 Adding a New GPU Driver

### Step 1: Create ASM Functions

```nasm
; asm/amd_gpu.s

section .data
    ; Register offsets - defined per GPU family (example: RDNA2)
    AMD_GPU_ID_OFFSET   equ 0x0000    ; GPU identification register
    EXPECTED_ID         equ 0x73BF    ; Example: Navi 21 (RX 6800 XT)

section .text

global asm_amd_gpu_init
global asm_amd_submit_gfx
; ... etc

asm_amd_gpu_init:
    ; RCX = mmio_base
    ; RDX = doorbell_base
    
    ; Read GPU ID from MMIO space
    mov     eax, [rcx + AMD_GPU_ID_OFFSET]
    cmp     eax, EXPECTED_ID
    jne     .init_failed
    
    ; ... initialization sequence (power up, reset, enable) ...
    
    xor     eax, eax    ; Success
    ret
    
.init_failed:
    mov     eax, 1      ; Error
    ret
```

### Step 2: Create Rust Bindings

```rust
// src/device/amd_gpu/bindings.rs

extern "win64" {
    fn asm_amd_gpu_init(mmio_base: u64, doorbell_base: u64) -> u32;
    fn asm_amd_submit_gfx(ring: *mut AmdRingState, cmd: *const u32, dwords: u32) -> u32;
    // ... etc
}
```

### Step 3: Implement DisplayDriver Trait

```rust
// src/device/amd_gpu/driver.rs

pub struct AmdGpuDriver {
    mmio_base: u64,
    doorbell_base: u64,
    gfx_ring: AmdRingState,
    // ... state
}

impl DisplayDriver for AmdGpuDriver {
    fn capabilities(&self) -> &DisplayCapabilities {
        &self.caps
    }
    
    fn set_mode(&mut self, width: u32, height: u32, format: PixelFormat) -> Result<(), DisplayError> {
        let result = unsafe { asm_amd_set_mode(self.mmio_base, 0, width, height) };
        if result != 0 {
            return Err(DisplayError::ModeSetFailed);
        }
        Ok(())
    }
    
    // ... implement other methods
}
```

### Step 4: Register Driver

```rust
// Add to DRIVER_REGISTRY
DriverRegistration {
    name: "amd_rdna2",
    vendor_id: 0x1002,
    device_ids: &[...],
    class_code: 0x030000,
    create: |device, dma| {
        let driver = AmdGpuDriver::new(device, dma)?;
        Ok(Box::new(driver))
    },
    priority: 100,
},
```

---

# 9. Memory Management

## 9.1 GPU Memory Types

| Type | Description | Mapping |
|------|-------------|---------|
| VRAM | Video RAM on GPU | Mapped via BAR |
| GTT | Graphics Translation Table | System memory via GART |
| System | System memory for DMA | Identity-mapped |

## 9.2 Resource Allocation

```rust
/// GPU resource allocation
pub trait GpuAllocator {
    /// Allocate VRAM
    fn alloc_vram(&mut self, size: u64, align: u64) -> Result<GpuAddr, AllocError>;
    
    /// Allocate GTT-mapped system memory
    fn alloc_gtt(&mut self, size: u64, align: u64) -> Result<(GpuAddr, CpuAddr), AllocError>;
    
    /// Free allocation
    fn free(&mut self, addr: GpuAddr);
}
```

## 9.3 Address Translation

```rust
/// Address translation for GPU
pub struct GpuAddressSpace {
    /// VRAM base (from BAR)
    vram_base: u64,
    /// VRAM size
    vram_size: u64,
    /// GTT base
    gtt_base: u64,
    /// GTT entries
    gtt_entries: &'static mut [u64],
}

impl GpuAddressSpace {
    /// Translate CPU address to GPU address
    pub fn cpu_to_gpu(&self, cpu_addr: u64) -> Option<u64> {
        // Identity map for system memory
        if cpu_addr < self.gtt_base {
            return Some(cpu_addr);
        }
        // GTT lookup for mapped memory
        let gtt_idx = ((cpu_addr - self.gtt_base) / PAGE_SIZE) as usize;
        if gtt_idx < self.gtt_entries.len() {
            return Some(self.gtt_entries[gtt_idx] & !0xFFF);
        }
        None
    }
}
```

---

*End of Hardware Extension Guide*
