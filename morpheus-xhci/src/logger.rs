//! Logger hook. Higher-level crate (typically hal-x86_64's serial UART)
//! installs a sink so xHCI code can emit diagnostics without depending on
//! hal-x86_64 — that back-edge would cycle through hal-x86_64's UsbHost impl.
//!
//! No allocation, no formatting — caller passes preformatted `&'static str`.

use core::sync::atomic::{AtomicPtr, Ordering};

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Info,
    Ok,
    Warn,
    Error,
}

pub type LogFn = fn(level: LogLevel, component: &'static str, code: u16, msg: &'static str);

// AtomicPtr<()> rather than AtomicPtr<fn(..)> — `fn` pointers don't satisfy
// AtomicPtr's bound. Cast at install / load time.
static LOGGER: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Install the global logger sink. Idempotent — last writer wins.
pub fn install(f: LogFn) {
    LOGGER.store(f as *mut (), Ordering::Release);
}

#[inline]
fn current() -> Option<LogFn> {
    let p = LOGGER.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        // SAFETY: we only ever store values produced from `LogFn` casts in
        // `install`; the inverse cast back to the same function-pointer type
        // is well-defined and the function itself has 'static lifetime.
        Some(unsafe { core::mem::transmute::<*mut (), LogFn>(p) })
    }
}

#[inline]
pub fn log(level: LogLevel, component: &'static str, code: u16, msg: &'static str) {
    if let Some(f) = current() {
        f(level, component, code, msg);
    }
}

#[inline]
pub fn info(component: &'static str, code: u16, msg: &'static str) {
    log(LogLevel::Info, component, code, msg);
}
#[inline]
pub fn ok(component: &'static str, code: u16, msg: &'static str) {
    log(LogLevel::Ok, component, code, msg);
}
#[inline]
pub fn warn(component: &'static str, code: u16, msg: &'static str) {
    log(LogLevel::Warn, component, code, msg);
}
#[inline]
pub fn error(component: &'static str, code: u16, msg: &'static str) {
    log(LogLevel::Error, component, code, msg);
}
