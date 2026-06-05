//! The seam: installable platform hooks (byte sink, IRQ guard, clock, cpu id,
//! framebuffer mirror) and their setters/accessors, all stored as fn pointers in
//! `AtomicPtr` statics so they are reachable lock-free from any core.

use core::sync::atomic::{AtomicPtr, Ordering};

/// UART byte-out, installed by the platform HAL. `None` until set (UART output
/// skipped; ring + FB still work).
static BYTE_SINK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// The console CRLF-translates before calling, so the sink sees `\r` then `\n`.
pub fn set_byte_sink(f: fn(u8)) {
    BYTE_SINK.store(f as *mut (), Ordering::Release);
}

#[inline]
pub(crate) fn byte_sink() -> Option<fn(u8)> {
    let p = BYTE_SINK.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        // SAFETY: BYTE_SINK only ever holds a `fn(u8)` cast via set_byte_sink,
        // or null (handled above).
        Some(unsafe { core::mem::transmute::<*mut (), fn(u8)>(p) })
    }
}

/// IRQ save/restore for same-core lock reentrancy. Default no-ops are correct
/// pre-SMP / IRQs-off early boot; platform installs `pushf;cli` / `popf`.
static IRQ_SAVE: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
static IRQ_RESTORE: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// `save` disables interrupts and returns prior state; `restore` re-applies it.
pub fn set_irq_guard(save: fn() -> u64, restore: fn(u64)) {
    IRQ_SAVE.store(save as *mut (), Ordering::Release);
    IRQ_RESTORE.store(restore as *mut (), Ordering::Release);
}

#[inline]
pub(crate) fn irq_save() -> u64 {
    let p = IRQ_SAVE.load(Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    // SAFETY: IRQ_SAVE only ever holds a `fn() -> u64` cast via set_irq_guard.
    let f = unsafe { core::mem::transmute::<*mut (), fn() -> u64>(p) };
    f()
}

#[inline]
pub(crate) fn irq_restore(state: u64) {
    let p = IRQ_RESTORE.load(Ordering::Acquire);
    if p.is_null() {
        return;
    }
    // SAFETY: IRQ_RESTORE only ever holds a `fn(u64)` cast via set_irq_guard.
    let f = unsafe { core::mem::transmute::<*mut (), fn(u64)>(p) };
    f(state);
}

/// Uptime clock (µs). Default returns 0 so early lines stamp `[    0.000000]`.
static CLOCK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// `f` must return monotonic microseconds since boot.
pub fn set_clock(f: fn() -> u64) {
    CLOCK.store(f as *mut (), Ordering::Release);
}

#[inline]
pub(crate) fn clock_us() -> u64 {
    let p = CLOCK.load(Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    // SAFETY: CLOCK only ever holds a `fn() -> u64` cast via set_clock.
    let f = unsafe { core::mem::transmute::<*mut (), fn() -> u64>(p) };
    f()
}

/// Current-core index. Default returns 0 so pre-per-cpu lines show `[c0]`.
static CPU_ID: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// `f` must be callable from any context (installed only after per-cpu is ready).
pub fn set_cpu_id(f: fn() -> u32) {
    CPU_ID.store(f as *mut (), Ordering::Release);
}

#[inline]
pub(crate) fn cpu_id() -> u32 {
    let p = CPU_ID.load(Ordering::Acquire);
    if p.is_null() {
        return 0;
    }
    // SAFETY: CPU_ID only ever holds a `fn() -> u32` cast via set_cpu_id.
    let f = unsafe { core::mem::transmute::<*mut (), fn() -> u32>(p) };
    f()
}

/// FB mirror installed by the bootloader; `None` until the FB is up.
static FB_SINK: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

#[inline]
pub(crate) fn fb_sink() -> Option<unsafe fn(u8)> {
    let p = FB_SINK.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        // SAFETY: FB_SINK only ever holds an `unsafe fn(u8)` cast via the
        // setters below, or null (handled above).
        Some(unsafe { core::mem::transmute::<*mut (), unsafe fn(u8)>(p) })
    }
}

pub fn set_fb_sink(f: unsafe fn(u8)) {
    FB_SINK.store(f as *mut (), Ordering::Release);
}

/// Alias for the legacy HAL name.
pub fn set_live_console_hook(f: unsafe fn(u8)) {
    set_fb_sink(f);
}

pub fn clear_live_console_hook() {
    FB_SINK.store(core::ptr::null_mut(), Ordering::Release);
}
