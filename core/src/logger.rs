// Global logging system for Morpheus

use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_LOG_ENTRIES: usize = 512; // Increased from 64 to support more logs

static mut LOG_BUFFER: [Option<&'static str>; MAX_LOG_ENTRIES] = [None; MAX_LOG_ENTRIES];
static LOG_HEAD: AtomicUsize = AtomicUsize::new(0); // Next write position (head of ring)
static LOG_COUNT: AtomicUsize = AtomicUsize::new(0); // Total logs written

pub fn log(message: &'static str) {
    let count = LOG_COUNT.fetch_add(1, Ordering::SeqCst);
    let idx = count % MAX_LOG_ENTRIES; // Ring buffer wrap-around

    unsafe {
        LOG_BUFFER[idx] = Some(message);
    }

    LOG_HEAD.store((count + 1) % MAX_LOG_ENTRIES, Ordering::SeqCst);
}

/// Returns an iterator over all valid log entries in chronological order
/// The ring buffer maintains up to MAX_LOG_ENTRIES logs. When full, oldest logs are overwritten.
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

    // Calculate start index for reading
    let start_idx = if total_count >= MAX_LOG_ENTRIES {
        // Buffer has wrapped, start from oldest entry
        total_count % MAX_LOG_ENTRIES
    } else {
        // Buffer hasn't wrapped yet, start from beginning
        0
    };

    LogIterator {
        start_idx,
        current: 0,
        remaining: num_logs,
    }
}

/// Get the last N log entries (up to MAX_LOG_ENTRIES)
pub fn get_last_n_logs(n: usize) -> LogIterator {
    let total_count = LOG_COUNT.load(Ordering::SeqCst);
    let available = total_count.min(MAX_LOG_ENTRIES);
    let num_logs = n.min(available);

    let start_idx = if total_count >= MAX_LOG_ENTRIES {
        // Buffer has wrapped
        (total_count - num_logs) % MAX_LOG_ENTRIES
    } else {
        // Buffer hasn't wrapped
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

// Macro for easier logging
#[macro_export]
macro_rules! log_info {
    ($msg:expr) => {
        $crate::logger::log($msg)
    };
}
