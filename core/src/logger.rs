//! Ring-buffered static log of `&'static str` messages.

use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_LOG_ENTRIES: usize = 512;

static mut LOG_BUFFER: [Option<&'static str>; MAX_LOG_ENTRIES] = [None; MAX_LOG_ENTRIES];
static LOG_HEAD: AtomicUsize = AtomicUsize::new(0);
static LOG_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn log(message: &'static str) {
    let count = LOG_COUNT.fetch_add(1, Ordering::SeqCst);
    let idx = count % MAX_LOG_ENTRIES;

    unsafe {
        LOG_BUFFER[idx] = Some(message);
    }

    LOG_HEAD.store((count + 1) % MAX_LOG_ENTRIES, Ordering::SeqCst);
}

/// Iterates entries in chronological order; oldest are overwritten when full.
pub struct LogIterator {
    start_idx: usize,
    current: usize,
    remaining: usize,
}

impl Iterator for LogIterator {
    type Item = &'static str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let idx = (self.start_idx + self.current) % MAX_LOG_ENTRIES;
        self.current += 1;
        self.remaining -= 1;

        unsafe { LOG_BUFFER[idx] }
    }
}

pub fn get_logs_iter() -> LogIterator {
    let total_count = LOG_COUNT.load(Ordering::SeqCst);
    let num_logs = total_count.min(MAX_LOG_ENTRIES);

    let start_idx = if total_count >= MAX_LOG_ENTRIES {
        total_count % MAX_LOG_ENTRIES
    } else {
        0
    };

    LogIterator {
        start_idx,
        current: 0,
        remaining: num_logs,
    }
}

/// Last `n` entries, capped at `MAX_LOG_ENTRIES`.
pub fn get_last_n_logs(n: usize) -> LogIterator {
    let total_count = LOG_COUNT.load(Ordering::SeqCst);
    let available = total_count.min(MAX_LOG_ENTRIES);
    let num_logs = n.min(available);

    let start_idx = if total_count >= MAX_LOG_ENTRIES {
        (total_count - num_logs) % MAX_LOG_ENTRIES
    } else {
        total_count.saturating_sub(num_logs)
    };

    LogIterator {
        start_idx,
        current: 0,
        remaining: num_logs,
    }
}

pub fn log_count() -> usize {
    LOG_COUNT.load(Ordering::SeqCst).min(MAX_LOG_ENTRIES)
}

pub fn total_log_count() -> usize {
    LOG_COUNT.load(Ordering::SeqCst)
}

#[macro_export]
macro_rules! log_info {
    ($msg:expr) => {
        $crate::logger::log($msg)
    };
}
