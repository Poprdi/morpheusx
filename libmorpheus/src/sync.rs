//! Futex-based synchronization primitives.
//!
//! Mutex, Condvar, OnceLock, and an mpsc channel — all built on
//! SYS_FUTEX so the kernel does the scheduling instead of burning
//! cycles in a spin loop.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::raw::{syscall3, FUTEX_WAIT, FUTEX_WAKE, SYS_FUTEX};

// -- raw futex helpers --

/// Block if `*addr == expected`. Returns when woken or if the value changed.
#[inline]
fn futex_wait(addr: &AtomicU32, expected: u32) {
    unsafe {
        syscall3(
            SYS_FUTEX,
            addr as *const AtomicU32 as u64,
            FUTEX_WAIT,
            expected as u64,
        );
    }
}

/// Wake up to `count` waiters on `addr`. Returns number woken.
#[inline]
fn futex_wake(addr: &AtomicU32, count: u32) -> u32 {
    unsafe {
        syscall3(
            SYS_FUTEX,
            addr as *const AtomicU32 as u64,
            FUTEX_WAKE,
            count as u64,
        ) as u32
    }
}

// -----------------------------------------------------------------------
// Mutex<T>
// -----------------------------------------------------------------------

/// Futex-based mutex. No spinning, no alloc, no nonsense.
///
/// State word: 0 = unlocked, 1 = locked (no waiters), 2 = locked (waiters).
/// The 3-state protocol avoids thundering-herd wakes.
pub struct Mutex<T> {
    state: AtomicU32,
    data: UnsafeCell<T>,
}

// SAFETY: Mutex provides exclusive access — that's the whole point.
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(val: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            data: UnsafeCell::new(val),
        }
    }

    /// Acquire the lock. Blocks if contended.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        // Fast path: uncontended, 0 → 1.
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return MutexGuard { mutex: self };
        }

        // Slow path: set state to 2 (contended) and sleep.
        loop {
            // If state was already nonzero, swap to 2 to signal "waiters exist".
            let old = self.state.swap(2, Ordering::Acquire);
            if old == 0 {
                // We got it between the CAS failure and the swap. Lucky.
                return MutexGuard { mutex: self };
            }
            // Sleep until state != 2 (i.e. someone unlocked).
            futex_wait(&self.state, 2);
        }
    }

    /// Try to acquire without blocking. Returns None on contention.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }

    fn unlock_inner(&self) {
        // If state was 1 (no waiters), just set to 0.
        if self.state.swap(0, Ordering::Release) == 2 {
            // There were waiters — wake one.
            futex_wake(&self.state, 1);
        }
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> core::ops::Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> core::ops::DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock_inner();
    }
}

// -----------------------------------------------------------------------
// Condvar
// -----------------------------------------------------------------------

/// Condition variable. Pair with a Mutex for classic wait/notify.
pub struct Condvar {
    seq: AtomicU32,
}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            seq: AtomicU32::new(0),
        }
    }

    /// Block until notified. Releases the mutex, sleeps, re-acquires.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let seq = self.seq.load(Ordering::Relaxed);
        let mutex = guard.mutex;
        // Drop the guard (releases the lock) BEFORE sleeping.
        core::mem::drop(guard);
        // Sleep until seq changes (i.e. someone called notify).
        futex_wait(&self.seq, seq);
        // Re-acquire.
        mutex.lock()
    }

    /// Wake one waiter.
    pub fn notify_one(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        futex_wake(&self.seq, 1);
    }

    /// Wake all waiters.
    pub fn notify_all(&self) {
        self.seq.fetch_add(1, Ordering::Release);
        futex_wake(&self.seq, u32::MAX);
    }
}

// -----------------------------------------------------------------------
// OnceLock
// -----------------------------------------------------------------------

/// Run-once initialization. Like std::sync::OnceLock.
///
/// States: 0 = empty, 1 = initializing, 2 = ready.
pub struct OnceLock<T> {
    state: AtomicU32,
    data: UnsafeCell<Option<T>>,
}

unsafe impl<T: Send + Sync> Send for OnceLock<T> {}
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> OnceLock<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU32::new(0),
            data: UnsafeCell::new(None),
        }
    }

    /// Get the value, initializing it with `f` if this is the first call.
    pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        // Fast path: already initialized.
        if self.state.load(Ordering::Acquire) == 2 {
            return unsafe { (*self.data.get()).as_ref().unwrap_unchecked() };
        }

        // Try to claim the initializer slot.
        if self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            unsafe { *self.data.get() = Some(f()) };
            self.state.store(2, Ordering::Release);
            futex_wake(&self.state, u32::MAX);
        } else {
            // Someone else is initializing — wait for them.
            while self.state.load(Ordering::Acquire) != 2 {
                futex_wait(&self.state, 1);
            }
        }
        unsafe { (*self.data.get()).as_ref().unwrap_unchecked() }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == 2 {
            Some(unsafe { (*self.data.get()).as_ref().unwrap_unchecked() })
        } else {
            None
        }
    }
}
