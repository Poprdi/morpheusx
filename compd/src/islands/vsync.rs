use crate::islands::CompState;

// Single-core: vsync is approximated by wall-clock polling; real cadence
// comes from SYS_YIELD + scheduler. TODO: hook hardware vsync via SYS_IRQ_ATTACH.

#[allow(unused)]
const TARGET_FRAME_NS: u64 = 16_666_667; // ~60 Hz

#[inline]
pub fn tick(_state: &mut CompState) {
    let _ = libmorpheus::time::clock_gettime();
}
