use crate::islands::CompState;

// on single-core: vsync is approximated by wall-clock polling.
// the real gate is SYS_YIELD + scheduler cadence.
// future: hook hardware vsync IRQ via SYS_IRQ_ATTACH.

#[allow(unused)]
const TARGET_FRAME_NS: u64 = 16_666_667; // ~60 Hz

#[inline]
pub fn tick(_state: &mut CompState) {
    let _ = libmorpheus::time::clock_gettime();
}
