//! Morpheus Display Crate
//!
//! Provides framebuffer-based text output that works both pre and post ExitBootServices.
//! Uses standalone ASM for all hardware-facing framebuffer operations.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │           TextOutput Trait                   │
//! │  (reset, clear, write_char, write_str, ...) │
//! └─────────────────────────────────────────────┘
//!                      │
//!         ┌────────────┴────────────┐
//!         ▼                         ▼
//! ┌───────────────┐         ┌───────────────┐
//! │ UefiTextOutput│         │ FbTextOutput  │
//! │  (pre-EBS)    │         │  (post-EBS)   │
//! └───────────────┘         └───────┬───────┘
//!         │                         │
//!         ▼                         ▼
//! ┌───────────────┐         ┌───────────────┐
//! │ UEFI Protocol │         │   Console     │
//! │  Passthrough  │         │  Framebuffer  │
//! └───────────────┘         └───────┬───────┘
//!                                   │
//!                                   ▼
//!                           ┌───────────────┐
//!                           │  ASM Layer    │
//!                           │ asm_fb_write32│
//!                           │ asm_fb_memset │
//!                           │ asm_fb_memcpy │
//!                           └───────────────┘
//! ```
//!
//! # Features
//!
//! - `uefi-backend`: Enable UEFI SimpleTextOutput passthrough (pre-EBS)
//! - `framebuffer-backend`: Enable raw framebuffer rendering via ASM (post-EBS)

#![no_std]
#![allow(dead_code)]

pub mod colors;
pub mod types;

// ASM bindings - always available for framebuffer-backend
#[cfg(feature = "framebuffer-backend")]
pub mod asm;

#[cfg(feature = "framebuffer-backend")]
pub mod framebuffer;

#[cfg(feature = "framebuffer-backend")]
pub mod font;

#[cfg(feature = "framebuffer-backend")]
pub mod console;

#[cfg(feature = "framebuffer-backend")]
pub mod fb_backend;

#[cfg(feature = "uefi-backend")]
pub mod uefi_backend;

pub mod global;

pub use colors::*;
pub use types::*;

/// Trait for text output backends.
///
/// This provides an EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL-compatible interface
/// that can be implemented by either:
/// - UefiTextOutput: Passthrough to UEFI firmware (pre-EBS)
/// - FbTextOutput: Raw framebuffer rendering via ASM (post-EBS)
pub trait TextOutput {
    /// Reset the output device.
    fn reset(&mut self);

    /// Clear the screen.
    fn clear(&mut self);

    /// Set cursor position (0-indexed).
    fn set_cursor(&mut self, col: usize, row: usize);

    /// Set text attribute (EFI color encoding: fg bits 0-3, bg bits 4-6).
    fn set_attribute(&mut self, attr: u8);

    /// Write a single character at current cursor position.
    fn write_char(&mut self, c: char);

    /// Write a string at current cursor position.
    fn write_str(&mut self, s: &str);

    /// Get number of columns.
    fn cols(&self) -> usize;

    /// Get number of rows.
    fn rows(&self) -> usize;

    /// Enable or disable cursor visibility.
    fn enable_cursor(&mut self, _visible: bool) {}
}
