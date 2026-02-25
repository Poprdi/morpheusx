//! Futex-based synchronization primitives.
//!
//! Mutex, Condvar, OnceLock, RwLock, and an mpsc channel — all built on
//! SYS_FUTEX so the kernel does the scheduling instead of burning
//! cycles in a spin loop.

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::sync::Arc;
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

// -----------------------------------------------------------------------
// RwLock<T>
// -----------------------------------------------------------------------

/// Reader-writer lock.  Multiple readers or one writer.
///
/// State word encoding:
/// - `0` = unlocked
/// - `1..=0x7FFF_FFFE` = N readers holding the lock
/// - `0xFFFF_FFFF` = writer holding the lock
///
/// Waiters spin briefly then park on the futex.
pub struct RwLock<T> {
    state: AtomicU32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

const UNLOCKED: u32 = 0;
const WRITER: u32 = 0xFFFF_FFFF;

impl<T> RwLock<T> {
    pub const fn new(val: T) -> Self {
        Self {
            state: AtomicU32::new(UNLOCKED),
            data: UnsafeCell::new(val),
        }
    }

    /// Acquire a shared (reader) lock.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s != WRITER {
                // Try to increment reader count.
                if self
                    .state
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return RwLockReadGuard { lock: self };
                }
            } else {
                // Writer holds it — park.
                futex_wait(&self.state, WRITER);
            }
        }
    }

    /// Acquire an exclusive (writer) lock.
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            if self
                .state
                .compare_exchange(UNLOCKED, WRITER, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return RwLockWriteGuard { lock: self };
            }
            let s = self.state.load(Ordering::Relaxed);
            futex_wait(&self.state, s);
        }
    }

    /// Try to acquire a read lock without blocking.
    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
        let s = self.state.load(Ordering::Relaxed);
        if s != WRITER {
            if self
                .state
                .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(RwLockReadGuard { lock: self });
            }
        }
        None
    }

    /// Try to acquire a write lock without blocking.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
        if self
            .state
            .compare_exchange(UNLOCKED, WRITER, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard { lock: self })
        } else {
            None
        }
    }
}

pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> core::ops::Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        let prev = self.lock.state.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            // We were the last reader — wake a waiting writer.
            futex_wake(&self.lock.state, 1);
        }
    }
}

pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> core::ops::Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> core::ops::DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.store(UNLOCKED, Ordering::Release);
        // Wake all waiters — readers and writers both.
        futex_wake(&self.lock.state, u32::MAX);
    }
}

// -----------------------------------------------------------------------
// mpsc — multi-producer, single-consumer channel
// -----------------------------------------------------------------------

/// Multi-producer, single-consumer channel.
///
/// Built on `Mutex<VecDeque<T>>` + `Condvar`.  Bounded (backpressure)
/// or unbounded.
///
/// # Example
/// ```ignore
/// use libmorpheus::sync::mpsc;
/// let (tx, rx) = mpsc::channel();
/// tx.send(42);
/// assert_eq!(rx.recv(), Some(42));
/// ```
pub mod mpsc {
    use super::*;

    struct Inner<T> {
        queue: Mutex<VecDeque<T>>,
        /// Notifies the receiver when items are available.
        not_empty: Condvar,
        /// Number of live senders.  When 0, receiver returns None.
        senders: AtomicU32,
    }

    /// Sending half.  Clone to get multiple producers.
    pub struct Sender<T> {
        inner: Arc<Inner<T>>,
    }

    /// Receiving half.
    pub struct Receiver<T> {
        inner: Arc<Inner<T>>,
    }

    // SAFETY: T: Send is required for cross-thread transfer.
    unsafe impl<T: Send> Send for Sender<T> {}
    unsafe impl<T: Send> Send for Receiver<T> {}
    unsafe impl<T: Send> Sync for Sender<T> {}

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            self.inner.senders.fetch_add(1, Ordering::Relaxed);
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    impl<T> Drop for Sender<T> {
        fn drop(&mut self) {
            if self.inner.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
                // Last sender — wake receiver so it sees None.
                self.inner.not_empty.notify_one();
            }
        }
    }

    impl<T> Sender<T> {
        /// Send a value.  Never blocks (unbounded queue).
        pub fn send(&self, val: T) {
            {
                let mut q = self.inner.queue.lock();
                q.push_back(val);
            }
            self.inner.not_empty.notify_one();
        }
    }

    impl<T> Receiver<T> {
        /// Blocking receive.  Returns `None` when all senders are dropped
        /// and the queue is empty.
        pub fn recv(&self) -> Option<T> {
            loop {
                {
                    let mut q = self.inner.queue.lock();
                    if let Some(val) = q.pop_front() {
                        return Some(val);
                    }
                    // Queue empty — check if all senders are gone.
                    if self.inner.senders.load(Ordering::Acquire) == 0 {
                        return None;
                    }
                }
                // Park until notified.  We need to release and re-acquire
                // the mutex via condvar.
                let guard = self.inner.queue.lock();
                if guard.is_empty() && self.inner.senders.load(Ordering::Acquire) > 0 {
                    let _guard = self.inner.not_empty.wait(guard);
                }
            }
        }

        /// Non-blocking try_recv.
        pub fn try_recv(&self) -> Option<T> {
            self.inner.queue.lock().pop_front()
        }

        /// Iterator that drains the channel until all senders are dropped.
        pub fn iter(&self) -> RecvIter<'_, T> {
            RecvIter { rx: self }
        }
    }

    pub struct RecvIter<'a, T> {
        rx: &'a Receiver<T>,
    }

    impl<T> Iterator for RecvIter<'_, T> {
        type Item = T;
        fn next(&mut self) -> Option<T> {
            self.rx.recv()
        }
    }

    /// Create an unbounded channel.
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let inner = Arc::new(Inner {
            queue: Mutex::new(VecDeque::new()),
            not_empty: Condvar::new(),
            senders: AtomicU32::new(1),
        });
        (
            Sender {
                inner: inner.clone(),
            },
            Receiver { inner },
        )
    }
}
