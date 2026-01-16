//! Synchronization Primitives
//!
//! Basic spinlocks and atomic operations for bare-metal use.
//! These are used internally by hwinit and exported for driver use.
//!
//! # Design Notes
//!
//! In a single-CPU bare-metal environment (which we are), spinlocks
//! primarily protect against interrupt reentrancy. With interrupts
//! disabled, a simple atomic flag suffices.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::cpu::idt::{disable_interrupts, enable_interrupts, interrupts_enabled};

// ═══════════════════════════════════════════════════════════════════════════
// SPINLOCK
// ═══════════════════════════════════════════════════════════════════════════

/// A simple spinlock.
///
/// Disables interrupts while held to prevent deadlock from interrupt handlers.
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// Safety: We ensure exclusive access through the lock
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// Create a new spinlock.
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, returns a guard that releases on drop.
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // Save and disable interrupts
        let interrupts_were_enabled = interrupts_enabled();
        disable_interrupts();

        // Spin until we acquire the lock
        while self.locked.compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_err() {
            // Spin hint
            core::hint::spin_loop();
        }

        SpinLockGuard {
            lock: self,
            interrupts_were_enabled,
        }
    }

    /// Try to acquire the lock without spinning.
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let interrupts_were_enabled = interrupts_enabled();
        disable_interrupts();

        if self.locked.compare_exchange(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok() {
            Some(SpinLockGuard {
                lock: self,
                interrupts_were_enabled,
            })
        } else {
            // Restore interrupts if we failed
            if interrupts_were_enabled {
                enable_interrupts();
            }
            None
        }
    }

    /// Check if the lock is currently held.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Get mutable access without locking (unsafe).
    ///
    /// # Safety
    /// Caller must ensure exclusive access.
    pub unsafe fn get_unchecked(&self) -> &mut T {
        &mut *self.data.get()
    }
}

/// Guard returned by SpinLock::lock()
pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    interrupts_were_enabled: bool,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);

        // Restore interrupt state
        if self.interrupts_were_enabled {
            enable_interrupts();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RAW SPINLOCK (no interrupt disable, for when you manage it yourself)
// ═══════════════════════════════════════════════════════════════════════════

/// Raw spinlock without interrupt management.
///
/// Use when you're already in an interrupt-disabled context or
/// don't need interrupt safety.
pub struct RawSpinLock {
    locked: AtomicBool,
}

impl RawSpinLock {
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) {
        while self.locked.compare_exchange_weak(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_err() {
            core::hint::spin_loop();
        }
    }

    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    pub fn try_lock(&self) -> bool {
        self.locked.compare_exchange(
            false,
            true,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ONCE (run-once initialization)
// ═══════════════════════════════════════════════════════════════════════════

/// Run-once initialization primitive.
pub struct Once {
    state: AtomicU64,
}

const ONCE_INCOMPLETE: u64 = 0;
const ONCE_RUNNING: u64 = 1;
const ONCE_COMPLETE: u64 = 2;

impl Once {
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(ONCE_INCOMPLETE),
        }
    }

    /// Run the closure exactly once.
    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
            return;
        }

        // Try to become the runner
        if self.state.compare_exchange(
            ONCE_INCOMPLETE,
            ONCE_RUNNING,
            Ordering::Acquire,
            Ordering::Relaxed,
        ).is_ok() {
            f();
            self.state.store(ONCE_COMPLETE, Ordering::Release);
        } else {
            // Someone else is running, wait for completion
            while self.state.load(Ordering::Acquire) != ONCE_COMPLETE {
                core::hint::spin_loop();
            }
        }
    }

    /// Check if initialization has completed.
    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == ONCE_COMPLETE
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LAZY (lazily initialized value)
// ═══════════════════════════════════════════════════════════════════════════

/// Lazily initialized value.
pub struct Lazy<T, F = fn() -> T> {
    once: Once,
    init: UnsafeCell<Option<F>>,
    value: UnsafeCell<Option<T>>,
}

unsafe impl<T: Send + Sync, F: Send> Sync for Lazy<T, F> {}

impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub const fn new(f: F) -> Self {
        Self {
            once: Once::new(),
            init: UnsafeCell::new(Some(f)),
            value: UnsafeCell::new(None),
        }
    }

    pub fn get(&self) -> &T {
        self.once.call_once(|| {
            let init = unsafe { (*self.init.get()).take().unwrap() };
            let value = init();
            unsafe { *self.value.get() = Some(value); }
        });

        unsafe { (*self.value.get()).as_ref().unwrap() }
    }
}

impl<T, F: FnOnce() -> T> Deref for Lazy<T, F> {
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERRUPT GUARD
// ═══════════════════════════════════════════════════════════════════════════

/// RAII guard that disables interrupts and restores on drop.
pub struct InterruptGuard {
    was_enabled: bool,
}

impl InterruptGuard {
    /// Disable interrupts, returning a guard that restores them on drop.
    pub fn new() -> Self {
        let was_enabled = interrupts_enabled();
        disable_interrupts();
        Self { was_enabled }
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            enable_interrupts();
        }
    }
}

/// Execute a closure with interrupts disabled.
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = InterruptGuard::new();
    f()
}
