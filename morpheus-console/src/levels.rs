//! Log-level lines (`log_info`/`ok`/`warn`/`error`), the ANSI color constants,
//! and the ANSI/code style toggles shared with the boot checklist.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::writer::line;

pub(crate) const ANSI_RESET: &str = "\x1b[0m";
pub(crate) const ANSI_CYAN: &str = "\x1b[36m";
pub(crate) const ANSI_GREEN: &str = "\x1b[32m";
pub(crate) const ANSI_YELLOW: &str = "\x1b[33m";
pub(crate) const ANSI_RED: &str = "\x1b[31m";

pub(crate) static LOG_ANSI_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_CODES_ENABLED: AtomicBool = AtomicBool::new(false);

/// `ansi=false` strips escapes; `codes=false` hides event numbers.
pub fn set_log_style(ansi_enabled: bool, codes_enabled: bool) {
    LOG_ANSI_ENABLED.store(ansi_enabled, Ordering::Release);
    LOG_CODES_ENABLED.store(codes_enabled, Ordering::Release);
}

#[inline]
fn log_level(color: &str, level: &str, component: &str, code: u16, msg: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    let codes = LOG_CODES_ENABLED.load(Ordering::Acquire);
    line(|w| {
        if ansi {
            w.str(color);
        }
        w.str("[");
        w.str(level);
        w.str("] [");
        w.str(component);
        if codes {
            w.str(":");
            w.dec_u16(code);
        }
        w.str("] ");
        w.str(msg);
        if ansi {
            w.str(ANSI_RESET);
        }
        w.str("\n");
    });
}

pub fn log_info(component: &str, code: u16, msg: &str) {
    log_level(ANSI_CYAN, "INFO", component, code, msg);
}

pub fn log_ok(component: &str, code: u16, msg: &str) {
    log_level(ANSI_GREEN, "OK", component, code, msg);
}

pub fn log_warn(component: &str, code: u16, msg: &str) {
    log_level(ANSI_YELLOW, "WARN", component, code, msg);
}

pub fn log_error(component: &str, code: u16, msg: &str) {
    log_level(ANSI_RED, "ERR", component, code, msg);
}
