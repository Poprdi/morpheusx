//! Spinlocks, once-init, RAII interrupt guard.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::intr::{disable_interrupts, enable_interrupts, interrupts_enabled};

#[inline(always)]
fn cas_acquire(lock: &AtomicBool) {
    while lock
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Disables IRQs while held. Default choice.
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: exclusive access enforced by the lock.
unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let interrupts_were_enabled = interrupts_enabled();
        disable_interrupts();

        cas_acquire(&self.locked);

        SpinLockGuard {
            lock: self,
            interrupts_were_enabled,
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let interrupts_were_enabled = interrupts_enabled();
        disable_interrupts();

        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard {
                lock: self,
                interrupts_were_enabled,
            })
        } else {
            if interrupts_were_enabled {
                enable_interrupts();
            }
            None
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// # Safety
    /// Caller guarantees exclusive access.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked(&self) -> &mut T {
        &mut *self.data.get()
    }
}

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
        if self.interrupts_were_enabled {
            enable_interrupts();
        }
    }
}

/// Caller owns IF state.
pub struct RawSpinLock {
    locked: AtomicBool,
}

impl Default for RawSpinLock {
    fn default() -> Self {
        Self::new()
    }
}

impl RawSpinLock {
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) {
        cas_acquire(&self.locked);
    }

    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    pub fn try_lock(&self) -> bool {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
}

/// IF save/restore in per-core slot indexed by `gs:[0x00]`. Cap 64 cores.
/// Pre-per-CPU, gs base is 0; BSP gets slot 0.
pub struct IsrSafeRawSpinLock {
    locked: AtomicBool,
    /// Only the holding core touches its slot.
    saved_if: [core::cell::UnsafeCell<bool>; 64],
}

// SAFETY: each core mutates a disjoint slot.
unsafe impl Sync for IsrSafeRawSpinLock {}
unsafe impl Send for IsrSafeRawSpinLock {}

impl Default for IsrSafeRawSpinLock {
    fn default() -> Self {
        Self::new()
    }
}

impl IsrSafeRawSpinLock {
    pub const fn new() -> Self {
        #[allow(clippy::declare_interior_mutable_const)]
        const CELL_FALSE: core::cell::UnsafeCell<bool> = core::cell::UnsafeCell::new(false);
        Self {
            locked: AtomicBool::new(false),
            saved_if: [CELL_FALSE; 64],
        }
    }

    #[inline(always)]
    fn core_index() -> usize {
        let idx: u32;
        unsafe {
            core::arch::asm!(
                "mov {0:e}, gs:[0x00]",
                out(reg) idx,
                options(nostack, readonly, preserves_flags)
            );
        }
        (idx as usize) & 63
    }

    pub fn lock(&self) {
        let was_enabled = interrupts_enabled();
        disable_interrupts();

        cas_acquire(&self.locked);

        // Stamp IF only after we hold the lock.
        let ci = Self::core_index();
        unsafe {
            *self.saved_if[ci].get() = was_enabled;
        }
    }

    pub fn unlock(&self) {
        let ci = Self::core_index();
        let was_enabled = unsafe { *self.saved_if[ci].get() };
        self.locked.store(false, Ordering::Release);

        if was_enabled {
            enable_interrupts();
        }
    }

    pub fn try_lock(&self) -> bool {
        let was_enabled = interrupts_enabled();
        disable_interrupts();

        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let ci = Self::core_index();
            unsafe {
                *self.saved_if[ci].get() = was_enabled;
            }
            true
        } else {
            if was_enabled {
                enable_interrupts();
            }
            false
        }
    }
}

pub struct Once {
    state: AtomicU64,
}

const ONCE_INCOMPLETE: u64 = 0;
const ONCE_RUNNING: u64 = 1;
const ONCE_COMPLETE: u64 = 2;

impl Default for Once {
    fn default() -> Self {
        Self::new()
    }
}

impl Once {
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(ONCE_INCOMPLETE),
        }
    }

    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.state.load(Ordering::Acquire) == ONCE_COMPLETE {
            return;
        }

        if self
            .state
            .compare_exchange(
                ONCE_INCOMPLETE,
                ONCE_RUNNING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            f();
            self.state.store(ONCE_COMPLETE, Ordering::Release);
        } else {
            while self.state.load(Ordering::Acquire) != ONCE_COMPLETE {
                core::hint::spin_loop();
            }
        }
    }

    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == ONCE_COMPLETE
    }
}

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
            unsafe {
                *self.value.get() = Some(value);
            }
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

/// Disables IRQs on construct, restores prior IF on drop.
pub struct InterruptGuard {
    was_enabled: bool,
}

impl Default for InterruptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl InterruptGuard {
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

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = InterruptGuard::new();
    f()
}
