//! GOP (Graphics Output Protocol) query for framebuffer info.
//!
//! This module queries the UEFI GOP to get framebuffer information
//! that can be passed to the bare-metal display driver post-EBS.

use crate::{
    BootServices, GopPixelFormat, GraphicsOutputProtocol, EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
};

/// Framebuffer information from GOP.
#[derive(Debug, Clone, Copy, Default)]
pub struct GopFramebufferInfo {
    /// Physical base address of framebuffer
    pub base: u64,
    /// Total size of framebuffer in bytes
    pub size: usize,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Stride in bytes (pixels_per_scan_line * 4)
    pub stride: u32,
    /// Pixel format (0=RGBX, 1=BGRX)
    pub format: u32,
}

impl GopFramebufferInfo {
    /// Check if framebuffer info is valid.
    pub fn is_valid(&self) -> bool {
        self.base != 0 && self.width > 0 && self.height > 0 && self.stride > 0
    }
}

/// Query GOP protocol for framebuffer information.
///
/// # Safety
/// Must be called before ExitBootServices.
/// `bs` must point to valid UEFI BootServices.
pub unsafe fn query_gop(bs: &BootServices) -> Option<GopFramebufferInfo> {
    let mut gop_ptr: *mut () = core::ptr::null_mut();

    // Locate GOP protocol
    let status = (bs.locate_protocol)(
        &EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID,
        core::ptr::null(),
        &mut gop_ptr,
    );

    if status != 0 || gop_ptr.is_null() {
        return None;
    }

    let gop = gop_ptr as *mut GraphicsOutputProtocol;
    let mode_ptr = (*gop).mode;

    if mode_ptr.is_null() {
        return None;
    }

    let mode = &*mode_ptr;
    let info = &*mode.info;

    // Convert pixel format
    let format = match info.pixel_format {
        GopPixelFormat::Rgbx => 0,
        GopPixelFormat::Bgrx => 1,
        GopPixelFormat::BitMask => 2,
        GopPixelFormat::BltOnly => 3,
    };

    // Calculate stride: pixels_per_scan_line * bytes_per_pixel (4 for 32-bit)
    let stride = info.pixels_per_scan_line * 4;

    Some(GopFramebufferInfo {
        base: mode.frame_buffer_base,
        size: mode.frame_buffer_size,
        width: info.horizontal_resolution,
        height: info.vertical_resolution,
        stride,
        format,
    })
}
