//! Display output module for post-EBS framebuffer rendering.
//!
//! This module provides a static display instance that mirrors serial output
//! to the framebuffer when the `display` feature is enabled.

use spin::Mutex;

#[cfg(feature = "display")]
use morpheus_display::{fb_backend::FbTextOutput, FramebufferInfo, PixelFormat, TextOutput};

/// Static display instance (initialized from handoff).
#[cfg(feature = "display")]
static DISPLAY: Mutex<Option<FbTextOutput>> = Mutex::new(None);

/// Initialize the framebuffer display from handoff info.
///
/// # Safety
/// Must be called after handoff validation, with valid framebuffer address.
#[cfg(feature = "display")]
pub unsafe fn init_display(base: u64, width: u32, height: u32, stride: u32, format: u32) {
    if base == 0 || width == 0 || height == 0 {
        return;
    }

    let fb_info = FramebufferInfo {
        base,
        size: (stride * height) as usize,
        width,
        height,
        stride,
        format: match format {
            0 => PixelFormat::Rgbx,
            1 => PixelFormat::Bgrx,
            _ => PixelFormat::Bgrx,
        },
    };

    let mut display = FbTextOutput::new(fb_info);
    display.clear();

    *DISPLAY.lock() = Some(display);
}

/// Write a string to the display (if initialized).
#[cfg(feature = "display")]
pub fn display_write(s: &str) {
    if let Some(ref mut display) = *DISPLAY.lock() {
        display.write_str(s);
    }
}

/// Write a string with newline to the display.
#[cfg(feature = "display")]
pub fn display_writeln(s: &str) {
    if let Some(ref mut display) = *DISPLAY.lock() {
        display.write_str(s);
        display.write_str("\n");
    }
}

/// Check if display is initialized.
#[cfg(feature = "display")]
pub fn display_available() -> bool {
    DISPLAY.lock().is_some()
}

// Stubs when display feature is disabled
#[cfg(not(feature = "display"))]
pub unsafe fn init_display(_base: u64, _width: u32, _height: u32, _stride: u32, _format: u32) {}

#[cfg(not(feature = "display"))]
pub fn display_write(_s: &str) {}

#[cfg(not(feature = "display"))]
pub fn display_writeln(_s: &str) {}

#[cfg(not(feature = "display"))]
pub fn display_available() -> bool {
    false
}
