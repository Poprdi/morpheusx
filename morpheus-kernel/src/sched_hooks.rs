//! FB-policy callbacks the bootloader installs so the scheduler can coordinate
//! frame present + compositor activity each tick. Plus [`LoadedElfImage`].

use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};

type FbPresentTickFn = unsafe extern "C" fn();
type CompositorActiveFn = unsafe extern "C" fn() -> bool;
type ReleaseFbLockFn = unsafe extern "C" fn(pid: u32);

/// Result of loading an ELF into a fresh user address space.
pub struct LoadedElfImage {
    pub entry: u64,
    pub pml4_phys: u64,
    /// `(vaddr_base, phys_base, memsz_bytes)` per segment; seeds the per-process VMA table.
    pub segments: Vec<(u64, u64, u64)>,
}

static FB_PRESENT_TICK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static COMPOSITOR_ACTIVE: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static RELEASE_FB_LOCK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Install the framebuffer-tick callback (called from `scheduler_tick` on BSP).
pub fn install_fb_present_tick(f: FbPresentTickFn) {
    FB_PRESENT_TICK.store(f as *mut (), Ordering::Release);
}

pub fn install_compositor_active(f: CompositorActiveFn) {
    COMPOSITOR_ACTIVE.store(f as *mut (), Ordering::Release);
}

pub fn install_release_fb_lock_if_holder(f: ReleaseFbLockFn) {
    RELEASE_FB_LOCK.store(f as *mut (), Ordering::Release);
}

#[inline]
pub unsafe fn fb_present_tick() {
    let p = FB_PRESENT_TICK.load(Ordering::Acquire);
    if !p.is_null() {
        let f: FbPresentTickFn = core::mem::transmute(p);
        f();
    }
}

/// False before the bootloader installs a hook (pre-handoff, direct FB console).
#[inline]
pub unsafe fn compositor_active() -> bool {
    let p = COMPOSITOR_ACTIVE.load(Ordering::Acquire);
    if p.is_null() {
        return false;
    }
    let f: CompositorActiveFn = core::mem::transmute(p);
    f()
}

/// Release the FB lock if `pid` is the current holder (no-op if unset).
#[inline]
pub unsafe fn release_fb_lock_if_holder(pid: u32) {
    let p = RELEASE_FB_LOCK.load(Ordering::Acquire);
    if !p.is_null() {
        let f: ReleaseFbLockFn = core::mem::transmute(p);
        f(pid);
    }
}
