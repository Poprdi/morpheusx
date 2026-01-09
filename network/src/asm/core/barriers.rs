//! Memory barrier bindings.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.1, §2.4

// TODO: Implement extern declarations and safe wrappers
//
// extern "win64" {
//     pub fn asm_bar_sfence();
//     pub fn asm_bar_lfence();
//     pub fn asm_bar_mfence();
// }
//
// #[inline]
// pub fn sfence() {
//     unsafe { asm_bar_sfence(); }
// }
//
// #[inline]
// pub fn lfence() {
//     unsafe { asm_bar_lfence(); }
// }
//
// #[inline]
// pub fn mfence() {
//     unsafe { asm_bar_mfence(); }
// }
