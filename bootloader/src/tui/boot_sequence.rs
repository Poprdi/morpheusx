use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGRAY, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW,
};
use alloc::boxed::Box;
use core::cell::RefCell;
use morpheus_core::logger;

// Note: Network initialization moved to post-ExitBootServices flow.
// The following imports are kept for potential future use but init_network() is deprecated.
#[allow(unused_imports)]
use morpheus_core::net::{
    error_log_available, error_log_clear, error_log_pop, ErrorLogEntry, InitConfig, NetworkInit,
    NetworkInitResult, NetworkStatus,
};

/// Result of network initialization for bootstrap phase
///
/// DEPRECATED: Network initialization now happens post-ExitBootServices.
/// This enum is kept for API compatibility but Success variant won't be returned.
pub enum NetworkBootResult {
    /// Network initialized successfully (no longer used)
    Success(NetworkInitResult),
    /// Network initialization failed (no longer used)
    Failed,
    /// Network initialization skipped/deferred to download time
    Skipped,
}

pub struct BootSequence {
    last_rendered_count: usize,
    last_total_count: usize,
    completed: bool,
}

impl BootSequence {
    pub fn new() -> Self {
        Self {
            last_rendered_count: 0,
            last_total_count: 0,
            completed: false,
        }
    }

    pub fn mark_complete(&mut self) {
        self.completed = true;
    }

    pub fn render(&mut self, screen: &mut Screen, x: usize, y: usize) {
        let total_count = logger::total_log_count();

        // Only show last 20 logs to fit on screen
        let logs_to_show = 20;

        // Only re-render if logs have changed
        if total_count != self.last_total_count {
            // Collect logs into a buffer to compare
            let mut line_idx = 0;
            for log in logger::get_last_n_logs(logs_to_show) {
                let line_y = y + line_idx;
                if line_y < screen.height() {
                    // Clear just this line before writing
                    screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
                    screen.put_str_at(x, line_y, "[  OK  ] ", EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(x + 9, line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
                }
                line_idx += 1;
            }

            // Clear any remaining lines if we have fewer logs now
            let current_log_count = logger::log_count().min(logs_to_show);
            for i in current_log_count..logs_to_show {
                let line_y = y + i;
                if line_y < screen.height() {
                    screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
                }
            }

            self.last_total_count = total_count;
        }

        if self.completed {
            let displayed_logs = logs_to_show.min(logger::log_count());
            let final_y = y + displayed_logs;
            if final_y < screen.height() - 1 {
                screen.put_str_at(
                    x,
                    final_y + 1,
                    "> System initialized. Press any key...",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
            }
        }

        self.last_rendered_count = total_count;
    }

    /// Initialize network stack and display result.
    ///
    /// DEPRECATED: Network initialization now happens post-ExitBootServices.
    /// This function is kept for API compatibility but always returns Skipped.
    ///
    /// The new flow is:
    /// 1. Bootstrap completes without network
    /// 2. User browses catalog (static data, no network needed)
    /// 3. User selects ISO and confirms download
    /// 4. ExitBootServices is called
    /// 5. Bare-metal network stack initializes (VirtIO + smoltcp)
    /// 6. Download proceeds, then system reboots
    ///
    /// # Arguments
    /// * `_screen` - Screen for rendering status (unused)
    /// * `_x`, `_y` - Position for boot sequence display (unused)
    /// * `_get_time_ms` - Function returning current time (unused)
    ///
    /// # Returns
    /// Always returns `NetworkBootResult::Skipped`
    #[deprecated(note = "Network init moved to post-EBS. Use download commit flow instead.")]
    #[allow(unused_variables)]
    pub fn init_network(
        &mut self,
        _screen: &mut Screen,
        _x: usize,
        _y: usize,
        _get_time_ms: fn() -> u64,
    ) -> NetworkBootResult {
        // Network initialization is now deferred to download time
        // Just log that we're skipping it
        logger::log("Network: deferred to download time");
        NetworkBootResult::Skipped
    }

    /// LEGACY: Display network error logs below the boot sequence.
    ///
    /// DEPRECATED: No longer used since network init is post-EBS.
    #[deprecated(note = "Network init moved to post-EBS")]
    #[allow(dead_code)]
    fn display_network_errors(&self, _screen: &mut Screen, _x: usize, _y: usize) {
        // No-op - network errors now shown in bare-metal mode via serial
    }
}
