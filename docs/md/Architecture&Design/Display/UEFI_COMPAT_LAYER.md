# MorpheusX UEFI Display Compatibility Layer

**Version**: 1.0  
**Status**: AUTHORITATIVE  
**Date**: January 2026  

---

## Overview

This document defines the UEFI compatibility shim for the MorpheusX display stack. The shim provides backward compatibility with existing code that relies on the UEFI Graphics Output Protocol (GOP) while internally delegating to the new bare-metal driver registry.

---

## Table of Contents

1. [Purpose & Scope](#1-purpose--scope)
2. [API Equivalence](#2-api-equivalence)
3. [Compilation & Feature Flags](#3-compilation--feature-flags)
4. [Implementation Architecture](#4-implementation-architecture)
5. [Behavior Differences](#5-behavior-differences)
6. [Migration Guide](#6-migration-guide)
7. [Deprecation Strategy](#7-deprecation-strategy)

---

# 1. Purpose & Scope

## 1.1 Compatibility Goal

Preserve backward compatibility with existing code that:

1. Uses UEFI GOP for framebuffer access pre-ExitBootServices
2. Expects GOP-like BLT (Block Transfer) operations
3. Relies on UEFI-style mode enumeration
4. Uses UEFI pixel format conventions

## 1.2 Non-Goals

The compatibility layer does NOT:

1. Implement actual UEFI protocol interfaces (no EFI_GRAPHICS_OUTPUT_PROTOCOL)
2. Support runtime UEFI calls post-EBS
3. Provide 100% behavioral parity with all GOP implementations
4. Support GOP protocol chaining or driver binding

## 1.3 Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     LEGACY CODE                                 │
│                (uses GOP-style API)                             │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                 UEFI COMPATIBILITY SHIM                         │
│              (morpheus_display::compat)                         │
│                                                                 │
│   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐         │
│   │ UefiDisplay  │  │ BltOperation │  │ ModeInfo     │         │
│   │   Shim       │  │   Emulation  │  │   Query      │         │
│   └──────────────┘  └──────────────┘  └──────────────┘         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                NEW DISPLAY DRIVER LAYER                         │
│         (VirtIO-GPU, Vendor drivers, Framebuffer)               │
└─────────────────────────────────────────────────────────────────┘
```

---

# 2. API Equivalence

## 2.1 UEFI GOP to Shim Mapping

| UEFI GOP Function | Shim Equivalent | Notes |
|-------------------|-----------------|-------|
| `GOP->QueryMode()` | `shim.query_mode()` | Returns mode info |
| `GOP->SetMode()` | `shim.set_mode()` | May be restricted post-EBS |
| `GOP->Blt()` | `shim.blt()` | All BLT operations supported |
| `GOP->Mode->Info` | `shim.mode_info()` | Current mode information |
| `GOP->Mode->FrameBufferBase` | `shim.framebuffer_base()` | Physical address |
| `GOP->Mode->FrameBufferSize` | `shim.framebuffer_size()` | Size in bytes |

## 2.2 BLT Operations

The shim supports all standard BLT operations:

```rust
#[repr(u32)]
pub enum BltOperation {
    /// Fill rectangle with pixel
    VideoFill = 0,
    /// Copy from video to buffer
    VideoToBltBuffer = 1,
    /// Copy from buffer to video
    BufferToVideo = 2,
    /// Copy within video memory
    VideoToVideo = 3,
}
```

## 2.3 Pixel Format Mapping

| UEFI Format | Internal Format | Notes |
|-------------|-----------------|-------|
| `PixelRedGreenBlueReserved8BitPerColor` | `R8G8B8A8` | RGBA order |
| `PixelBlueGreenRedReserved8BitPerColor` | `B8G8R8A8` | BGRA order (common) |
| `PixelBitMask` | Custom | Via mask specification |
| `PixelBltOnly` | N/A | Direct FB access disabled |

## 2.4 Shim Interface

```rust
/// UEFI-compatible display shim
pub struct UefiDisplayShim<'a> {
    driver: &'a mut dyn DisplayDriver,
    mode_info: ModeInfo,
    framebuffer: Option<*mut u8>,
}

impl<'a> UefiDisplayShim<'a> {
    /// Create shim wrapping a display driver
    pub fn new(driver: &'a mut dyn DisplayDriver) -> Self;
    
    /// Query available modes
    pub fn query_mode(&self, mode_number: u32) -> Result<ModeInfo, DisplayError>;
    
    /// Set display mode
    pub fn set_mode(&mut self, mode_number: u32) -> Result<(), DisplayError>;
    
    /// Perform BLT operation
    pub fn blt(
        &mut self,
        blt_buffer: Option<&mut [BltPixel]>,
        operation: BltOperation,
        source_x: usize,
        source_y: usize,
        dest_x: usize,
        dest_y: usize,
        width: usize,
        height: usize,
        delta: usize,
    ) -> Result<(), DisplayError>;
    
    /// Get current mode info
    pub fn mode_info(&self) -> &ModeInfo;
    
    /// Get framebuffer base address
    pub fn framebuffer_base(&self) -> Option<u64>;
    
    /// Get framebuffer size
    pub fn framebuffer_size(&self) -> usize;
}
```

---

# 3. Compilation & Feature Flags

## 3.1 Feature Flag: `uefi-compat`

The compatibility layer is gated behind a feature flag:

```toml
# Cargo.toml
[features]
default = []
uefi-compat = []
```

Usage:
```bash
# Without compatibility (new code only)
cargo build

# With compatibility layer
cargo build --features uefi-compat
```

## 3.2 Conditional Compilation

```rust
// In application code
#[cfg(feature = "uefi-compat")]
use morpheus_display::compat::UefiDisplayShim;

fn init_display(driver: &mut dyn DisplayDriver) {
    #[cfg(feature = "uefi-compat")]
    {
        let shim = UefiDisplayShim::new(driver);
        legacy_graphics_init(&shim);
    }
    
    #[cfg(not(feature = "uefi-compat"))]
    {
        // Direct driver usage
        driver.set_mode(1920, 1080, PixelFormat::BGRA8888);
    }
}
```

## 3.3 Deprecation Warnings

When `uefi-compat` is enabled, deprecation warnings are emitted:

```rust
#[cfg(feature = "uefi-compat")]
#[deprecated(since = "1.0.0", note = "Use DisplayDriver trait directly")]
pub fn blt(&mut self, ...) -> Result<(), DisplayError> {
    // Implementation
}
```

---

# 4. Implementation Architecture

## 4.1 Pre-EBS Path

Before ExitBootServices, the shim can use actual UEFI GOP:

```rust
impl<'a> UefiDisplayShim<'a> {
    /// Create from UEFI GOP protocol (pre-EBS only)
    pub fn from_gop(gop: &'a mut GraphicsOutput) -> Self {
        // Extract current mode info
        let info = gop.current_mode_info();
        let fb_base = gop.frame_buffer().as_mut_ptr();
        let fb_size = gop.frame_buffer().size();
        
        Self {
            inner: ShimInner::UefiGop { gop },
            mode_info: info.into(),
            framebuffer_base: fb_base as u64,
            framebuffer_size: fb_size,
        }
    }
}
```

## 4.2 Post-EBS Path

After ExitBootServices, the shim delegates to the new driver:

```rust
impl<'a> UefiDisplayShim<'a> {
    /// Create from bare-metal driver (post-EBS)
    pub fn from_driver(driver: &'a mut dyn DisplayDriver) -> Self {
        let caps = driver.capabilities();
        
        Self {
            inner: ShimInner::BareMetal { driver },
            mode_info: ModeInfo::from_capabilities(caps),
            framebuffer_base: driver.framebuffer_base(),
            framebuffer_size: driver.framebuffer_size(),
        }
    }
}
```

## 4.3 BLT Implementation

```rust
impl<'a> UefiDisplayShim<'a> {
    pub fn blt(
        &mut self,
        blt_buffer: Option<&mut [BltPixel]>,
        operation: BltOperation,
        source_x: usize,
        source_y: usize,
        dest_x: usize,
        dest_y: usize,
        width: usize,
        height: usize,
        delta: usize,
    ) -> Result<(), DisplayError> {
        match &mut self.inner {
            ShimInner::UefiGop { gop } => {
                // Direct GOP call (pre-EBS)
                gop.blt(blt_buffer, operation, source_x, source_y, 
                        dest_x, dest_y, width, height, delta)
                    .map_err(|_| DisplayError::BltFailed)
            }
            ShimInner::BareMetal { driver } => {
                // Translate to driver operations (post-EBS)
                match operation {
                    BltOperation::VideoFill => {
                        self.fill_rect(driver, blt_buffer, dest_x, dest_y, width, height)
                    }
                    BltOperation::BufferToVideo => {
                        self.copy_to_video(driver, blt_buffer.unwrap(), 
                                          source_x, source_y, dest_x, dest_y, 
                                          width, height, delta)
                    }
                    BltOperation::VideoToBltBuffer => {
                        self.copy_from_video(driver, blt_buffer.unwrap(),
                                            source_x, source_y, dest_x, dest_y,
                                            width, height, delta)
                    }
                    BltOperation::VideoToVideo => {
                        self.copy_video_to_video(driver, source_x, source_y,
                                                dest_x, dest_y, width, height)
                    }
                }
            }
        }
    }
    
    fn copy_to_video(
        &mut self,
        driver: &mut dyn DisplayDriver,
        buffer: &[BltPixel],
        src_x: usize, src_y: usize,
        dst_x: usize, dst_y: usize,
        width: usize, height: usize,
        delta: usize,
    ) -> Result<(), DisplayError> {
        // Convert BltPixel to driver format
        let converted = self.convert_pixels(buffer, width, height, delta);
        
        // Create temporary resource
        let resource = driver.create_resource(width as u32, height as u32, 
                                              PixelFormat::BGRA8888)?;
        
        // Upload pixel data
        driver.upload_resource(resource, &converted)?;
        
        // Blit to scanout
        driver.blit_to_scanout(resource, dst_x as u32, dst_y as u32)?;
        
        // Flush
        driver.flush()?;
        
        // Cleanup
        driver.destroy_resource(resource)?;
        
        Ok(())
    }
}
```

---

# 5. Behavior Differences

## 5.1 Mode Setting

| Aspect | UEFI GOP | Shim (Post-EBS) |
|--------|----------|-----------------|
| Mode enumeration | Dynamic from device | Fixed set from driver capabilities |
| Mode switching | Immediate | May require fence wait |
| Failure handling | Returns EFI_STATUS | Returns DisplayError |
| Concurrent access | Undefined | Single-threaded only |

## 5.2 BLT Performance

| Operation | UEFI GOP | Shim (VirtIO-GPU) |
|-----------|----------|-------------------|
| VideoFill | Hardware-dependent | GPU command submission |
| BufferToVideo | May use DMA | Resource upload + flush |
| VideoToBltBuffer | May use DMA | Resource readback (slow) |
| VideoToVideo | Hardware blit | GPU copy command |

**Note**: VideoToBltBuffer is significantly slower via VirtIO-GPU as it requires GPU→host transfer.

## 5.3 Error Codes

| UEFI Status | Shim Error |
|-------------|------------|
| `EFI_SUCCESS` | `Ok(())` |
| `EFI_INVALID_PARAMETER` | `DisplayError::InvalidParameter` |
| `EFI_UNSUPPORTED` | `DisplayError::Unsupported` |
| `EFI_NOT_READY` | `DisplayError::NotReady` |
| `EFI_DEVICE_ERROR` | `DisplayError::DeviceError` |
| `EFI_OUT_OF_RESOURCES` | `DisplayError::ResourceExhausted` |

---

# 6. Migration Guide

## 6.1 Gradual Migration Path

### Phase 1: Add Shim Wrapper

```rust
// Before: Direct GOP usage
let gop = boot_services.locate_protocol::<GraphicsOutput>()?;
gop.blt(&mut buffer, BltOperation::BufferToVideo, ...)?;

// After Phase 1: Shim wrapper
let shim = UefiDisplayShim::from_gop(gop);
shim.blt(&mut buffer, BltOperation::BufferToVideo, ...)?;
```

### Phase 2: Abstract Over Shim

```rust
// After Phase 2: Use trait object
fn render(display: &mut dyn DisplayAdapter) {
    display.draw_buffer(&buffer, x, y);
}

// Can use shim or new API
render(&mut UefiDisplayShim::from_driver(driver));
// OR
render(&mut DirectDisplayAdapter::new(driver));
```

### Phase 3: Remove Shim Usage

```rust
// After Phase 3: Direct driver usage
driver.set_mode(1920, 1080, PixelFormat::BGRA8888)?;
let fb = driver.acquire_frame_buffer()?;
render_to_buffer(&mut fb);
driver.upload_resource(fb)?;
driver.flush()?;
```

## 6.2 Code Changes Required

### BLT to Resource Upload

```rust
// Old (GOP BLT)
gop.blt(&mut pixels, BltOperation::BufferToVideo, 
        0, 0, x, y, width, height, stride)?;

// New (Direct driver)
let resource = driver.create_resource(width, height, format)?;
driver.upload_resource(resource, &pixels)?;
driver.set_scanout(0, resource)?;
driver.flush()?;
```

### Mode Query to Capabilities

```rust
// Old (GOP mode query)
let mode_count = gop.mode().max_mode();
for i in 0..mode_count {
    let info = gop.query_mode(i)?;
    println!("Mode {}: {}x{}", i, info.width(), info.height());
}

// New (Driver capabilities)
let caps = driver.capabilities();
println!("Max: {}x{}", caps.max_width, caps.max_height);
println!("3D: {}", caps.has_3d);
```

---

# 7. Deprecation Strategy

## 7.1 Timeline

| Version | Status | Changes |
|---------|--------|---------|
| 1.0 | Current | Shim available, warnings on use |
| 1.1 | Planned | Shim hidden behind feature flag |
| 2.0 | Planned | Shim removed from default builds |
| 3.0 | Planned | Shim code deleted |

## 7.2 Deprecation Warnings

```rust
#[cfg(feature = "uefi-compat")]
#[deprecated(
    since = "1.0.0",
    note = "Use DisplayDriver::upload_resource() instead of BLT. \
            See docs/md/Architecture&Design/Display/IMPLEMENTATION_GUIDE.md \
            for direct driver usage patterns."
)]
pub fn blt(&mut self, ...) -> Result<(), DisplayError> {
    // ...
}
```

## 7.3 Migration Lints

Custom lint to detect deprecated usage:

```rust
// In build.rs or as Clippy lint
// Warn when using UefiDisplayShim with new code
#![warn(deprecated_display_api)]
```

---

# Appendix A: Complete Shim API Reference

```rust
/// UEFI-compatible display shim (deprecated, use DisplayDriver)
#[cfg(feature = "uefi-compat")]
pub struct UefiDisplayShim<'a> { /* ... */ }

impl<'a> UefiDisplayShim<'a> {
    /// Create shim from UEFI GOP protocol (pre-EBS only)
    pub fn from_gop(gop: &'a mut GraphicsOutput) -> Self;
    
    /// Create shim from bare-metal driver (post-EBS)
    pub fn from_driver(driver: &'a mut dyn DisplayDriver) -> Self;
    
    /// Query available display mode
    #[deprecated]
    pub fn query_mode(&self, mode_number: u32) -> Result<ModeInfo, DisplayError>;
    
    /// Set display mode
    #[deprecated]
    pub fn set_mode(&mut self, mode_number: u32) -> Result<(), DisplayError>;
    
    /// Perform BLT (Block Transfer) operation
    #[deprecated(note = "Use DisplayDriver::upload_resource()")]
    pub fn blt(
        &mut self,
        blt_buffer: Option<&mut [BltPixel]>,
        operation: BltOperation,
        source_x: usize,
        source_y: usize,
        dest_x: usize,
        dest_y: usize,
        width: usize,
        height: usize,
        delta: usize,
    ) -> Result<(), DisplayError>;
    
    /// Get current mode information
    pub fn mode_info(&self) -> &ModeInfo;
    
    /// Get framebuffer base physical address
    pub fn framebuffer_base(&self) -> Option<u64>;
    
    /// Get framebuffer size in bytes
    pub fn framebuffer_size(&self) -> usize;
    
    /// Check if direct framebuffer access is available
    pub fn has_direct_access(&self) -> bool;
}

/// BLT pixel format (UEFI-compatible)
#[repr(C)]
pub struct BltPixel {
    pub blue: u8,
    pub green: u8,
    pub red: u8,
    pub reserved: u8,
}

/// Display mode information (UEFI-compatible)
#[repr(C)]
pub struct ModeInfo {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: PixelFormat,
    pub pixel_information: PixelBitmask,
    pub pixels_per_scan_line: u32,
}
```

---

*End of UEFI Compatibility Layer Documentation*
