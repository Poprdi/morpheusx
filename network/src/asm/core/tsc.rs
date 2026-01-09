//! TSC (Time Stamp Counter) bindings.
//!
//! # Safety
//! TSC reads are always safe. Requires invariant TSC (verify via CPUID at boot).
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §2.2.1

// TODO: Implement extern declarations and safe wrappers
//
// extern "win64" {
//     pub fn asm_tsc_read() -> u64;
//     pub fn asm_tsc_read_serialized() -> u64;
// }
//
// #[inline]
// pub fn read_tsc() -> u64 {
//     unsafe { asm_tsc_read() }
// }
//
// #[inline]
// pub fn read_tsc_serialized() -> u64 {
//     unsafe { asm_tsc_read_serialized() }
// }
