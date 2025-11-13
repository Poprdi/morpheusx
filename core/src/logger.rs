// Global logging system for Morpheus

use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_LOG_ENTRIES: usize = 64;

static mut LOG_BUFFER: [Option<&'static str>; MAX_LOG_ENTRIES] = [None; MAX_LOG_ENTRIES];
static LOG_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn log(message: &'static str) {
    let idx = LOG_COUNT.fetch_add(1, Ordering::SeqCst);
    if idx < MAX_LOG_ENTRIES {
        unsafe {
            LOG_BUFFER[idx] = Some(message);
        }
    }
}

pub fn get_logs() -> &'static [Option<&'static str>] {
    let count = LOG_COUNT.load(Ordering::SeqCst).min(MAX_LOG_ENTRIES);
    unsafe { &LOG_BUFFER[..count] }
}

pub fn log_count() -> usize {
    LOG_COUNT.load(Ordering::SeqCst).min(MAX_LOG_ENTRIES)
}

// Macro for easier logging
#[macro_export]
macro_rules! log_info {
    ($msg:expr) => {
        $crate::logger::log($msg)
    };
}
