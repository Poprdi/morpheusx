//! Framebuffer text output for pre- and post-ExitBootServices.
//!
//! `uefi-backend` passes through SimpleTextOutput; `framebuffer-backend`
//! renders directly via standalone ASM (asm/fb.s).

#![no_std]
#![allow(dead_code)]
#![allow(clippy::missing_safety_doc)]

pub mod colors;
pub mod types;

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

/// EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL-compatible interface. Attribute uses
/// EFI encoding: fg in bits 0-3, bg in bits 4-6.
pub trait TextOutput {
    fn reset(&mut self);
    fn clear(&mut self);
    fn set_cursor(&mut self, col: usize, row: usize);
    fn set_attribute(&mut self, attr: u8);
    fn write_char(&mut self, c: char);
    fn write_str(&mut self, s: &str);
    fn cols(&self) -> usize;
    fn rows(&self) -> usize;
    fn enable_cursor(&mut self, _visible: bool) {}
}
