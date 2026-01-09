//! Cache management bindings.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §3.6 - Cache coherency

// TODO: Implement extern declarations
//
// extern "win64" {
//     pub fn asm_cache_clflush(addr: u64);
//     pub fn asm_cache_clflushopt(addr: u64);
// }
//
// /// Flush cache line containing address.
// ///
// /// # Safety
// /// Address must be valid.
// #[inline]
// pub unsafe fn clflush(addr: *const u8) {
//     asm_cache_clflush(addr as u64)
// }
