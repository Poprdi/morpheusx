//! Boot-chain title banner and the one-line-per-step `[ OK ]`/`[WARN]`/`[FAIL]`
//! checklist markers.

use core::sync::atomic::Ordering;

use crate::levels::{ANSI_CYAN, ANSI_GREEN, ANSI_RED, ANSI_RESET, ANSI_YELLOW, LOG_ANSI_ENABLED};
use crate::writer::{line, puts};

/// Boot-chain title banner; cyan when ANSI is enabled.
pub fn boot_banner(title: &str, version: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    puts("\n  ");
    if ansi {
        puts(ANSI_CYAN);
    }
    puts(title);
    if ansi {
        puts(ANSI_RESET);
    }
    puts("  ");
    puts(version);
    puts("\n\n");
}

/// One checklist row: `  [TAG]  label`. `tag` is 4-char status; `color` tints
/// only the bracketed tag.
fn boot_step(color: &str, tag: &str, label: &str) {
    let ansi = LOG_ANSI_ENABLED.load(Ordering::Acquire);
    line(|w| {
        w.str("  [");
        if ansi {
            w.str(color);
        }
        w.str(tag);
        if ansi {
            w.str(ANSI_RESET);
        }
        w.str("]  ");
        w.str(label);
        w.str("\n");
    });
}

/// Green `[ OK ]`.
pub fn boot_step_ok(label: &str) {
    boot_step(ANSI_GREEN, " OK ", label);
}

/// Yellow `[WARN]`; following `log_warn` lines carry the specifics.
pub fn boot_step_warn(label: &str) {
    boot_step(ANSI_YELLOW, "WARN", label);
}

/// Red `[FAIL]`.
pub fn boot_step_fail(label: &str) {
    boot_step(ANSI_RED, "FAIL", label);
}
