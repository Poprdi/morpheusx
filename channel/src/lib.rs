#![no_std]

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

/// SPSC ring buffer. `N` must be a power of two (const-asserted).
pub struct Channel<T, const N: usize> {
    buf: [UnsafeCell<MaybeUninit<T>>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

// Single-core scheduler: no preemption between send/recv within same process.
unsafe impl<T, const N: usize> Sync for Channel<T, N> {}
unsafe impl<T, const N: usize> Send for Channel<T, N> {}

impl<T, const N: usize> Channel<T, N> {
    // `N-1` mask requires power-of-two N.
    const ASSERT_POWER_OF_2: () = assert!(
        N.is_power_of_two(),
        "Channel capacity N must be a power of two"
    );

    pub const fn new() -> Self {
        let _ = Self::ASSERT_POWER_OF_2;
        // SAFETY: MaybeUninit arrays can be zero-initialized.
        Self {
            buf: unsafe { MaybeUninit::<[UnsafeCell<MaybeUninit<T>>; N]>::zeroed().assume_init() },
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Non-blocking. Returns `Err(msg)` if full.
    #[inline]
    pub fn send(&self, msg: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return Err(msg);
        }
        unsafe {
            (*self.buf[head & (N - 1)].get()).write(msg);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Non-blocking. Returns `None` if empty.
    #[inline]
    pub fn recv(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let msg = unsafe { (*self.buf[tail & (N - 1)].get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(msg)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        head.wrapping_sub(tail)
    }
}
