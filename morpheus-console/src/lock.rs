//! The per-line lock: cross-core mutual exclusion combined with the installed
//! IRQ guard, released in reverse order on drop.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::sink::{irq_restore, irq_save};

/// Per-line critical section. With the IRQ guard, replaces the HAL `SpinLock`
/// and is usable before `hal()`.
static LINE_LOCK: AtomicBool = AtomicBool::new(false);

/// Holds the lock + saved IRQ state; releases in reverse order on drop.
pub(crate) struct LineGuard {
    irq_state: u64,
}

impl LineGuard {
    #[inline]
    pub(crate) fn acquire() -> Self {
        let irq_state = irq_save();
        while LINE_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        LineGuard { irq_state }
    }
}

impl Drop for LineGuard {
    #[inline]
    fn drop(&mut self) {
        LINE_LOCK.store(false, Ordering::Release);
        irq_restore(self.irq_state);
    }
}
