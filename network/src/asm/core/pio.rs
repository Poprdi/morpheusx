//! Port I/O bindings.
//!
//! # Safety
//! Port must be valid I/O port for the operation.
//!
//! # Reference
//! ARCHITECTURE_V3.md - PIO layer

// TODO: Implement extern declarations and typed wrappers
//
// extern "win64" {
//     pub fn asm_pio_read8(port: u16) -> u8;
//     pub fn asm_pio_write8(port: u16, value: u8);
//     pub fn asm_pio_read16(port: u16) -> u16;
//     pub fn asm_pio_write16(port: u16, value: u16);
//     pub fn asm_pio_read32(port: u16) -> u32;
//     pub fn asm_pio_write32(port: u16, value: u32);
// }
