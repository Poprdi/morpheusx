use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGRAY, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW, EFI_DARKGREEN
};
use alloc::boxed::Box;
use core::cell::RefCell;
use morpheus_core::logger;

// Note: Network initialization moved to post-ExitBootServices flow.
// The following imports are kept for potential future use but init_network() is deprecated.
#[allow(unused_imports)]
use morpheus_core::net::{
    error_log_available, error_log_clear, error_log_pop, ErrorLogEntry, InitConfig, NetworkStatus,
};

/// Result of network initialization for bootstrap phase
///
/// DEPRECATED: Network initialization now happens post-ExitBootServices.
/// This enum is kept for API compatibility but Success variant won't be returned.
pub enum NetworkBootResult {
    /// Network initialization skipped/deferred to download time
    Skipped,
}

pub struct BootSequence {
    last_rendered_count: usize,
    last_total_count: usize,
    completed: bool,
    initialized: bool,
}

impl BootSequence {
    pub fn new() -> Self {
        Self {
            last_rendered_count: 0,
            last_total_count: 0,
            completed: false,
            initialized: false,
        }
    }

    pub fn mark_complete(&mut self) {
        self.completed = true;
    }

    /// ASCII logo for MorpheusX bootloader
    const LOGO: &'static [&'static str] = &[
        "███╗   ███╗ ██████╗ ██████╗ ██████╗ ██╗  ██╗███████╗██╗   ██╗███████╗██╗  ██╗",
        "████╗ ████║██╔═══██╗██╔══██╗██╔══██╗██║  ██║██╔════╝██║   ██║██╔════╝╚██╗██╔╝",
        "██╔████╔██║██║   ██║██████╔╝██████╔╝███████║█████╗  ██║   ██║███████╗ ╚███╔╝ ",
        "██║╚██╔╝██║██║   ██║██╔══██╗██╔═══╝ ██╔══██║██╔══╝  ██║   ██║╚════██║ ██╔██╗ ",
        "██║ ╚═╝ ██║╚██████╔╝██║  ██║██║     ██║  ██║███████╗╚██████╔╝███████║██╔╝ ██╗",
        "╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝  ╚═╝╚══════╝ ╚═════╝ ╚══════╝╚═╝  ╚═╝",
    ];

    /// Render the centered ASCII logo
    fn render_logo(&self, screen: &mut Screen, start_y: usize) {
        for (idx, line) in Self::LOGO.iter().enumerate() {
            let y = start_y + idx;
            if y < screen.height() {
                // Center the logo line
                let x = screen.center_x(line.chars().count());
                screen.put_str_at(x, y, line, EFI_DARKGREEN, EFI_BLACK);
            }
        }
    }

    pub fn render(&mut self, screen: &mut Screen, x: usize, y: usize) {
        // Initialize screen on first render
        if !self.initialized {
            screen.clear();
            self.initialized = true;
        }

        let total_count = logger::total_log_count();

        // Configuration
        let logs_to_show = 20;
        let logo_height = Self::LOGO.len();
        let spacing_after_logo = 2;
        
        // Calculate total content height (logo + spacing + logs + completion message)
        let total_content_height = logo_height + spacing_after_logo + logs_to_show + 3;
        
        // Center the entire content vertically
        let start_y = screen.center_y(total_content_height);
        
        // Render logo at the top of centered content
        self.render_logo(screen, start_y);
        
        // Calculate where logs should start
        let logs_start_y = start_y + logo_height + spacing_after_logo;

        // Only re-render if logs have changed
        if total_count != self.last_total_count {
            // Render logs centered
            let mut line_idx = 0;
            for log in logger::get_last_n_logs(logs_to_show) {
                let line_y = logs_start_y + line_idx;
                if line_y < screen.height() {
                    // Calculate content width for centering
                    let status_prefix = "[  OK  ] ";
                    let full_line_len = status_prefix.chars().count() + log.chars().count();
                    
                    // Center the log line
                    let centered_x = screen.center_x(full_line_len);
                    
                    // Clear the entire line first
                    let clear_str = " ".repeat(screen.width());
                    screen.put_str_at(0, line_y, &clear_str, EFI_BLACK, EFI_BLACK);
                    
                    // Render centered log line
                    screen.put_str_at(centered_x, line_y, status_prefix, EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(centered_x + status_prefix.chars().count(), line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
                }
                line_idx += 1;
            }

            // Clear any remaining lines if we have fewer logs now
            let current_log_count = logger::log_count().min(logs_to_show);
            for i in current_log_count..logs_to_show {
                let line_y = logs_start_y + i;
                if line_y < screen.height() {
                    let clear_str = " ".repeat(screen.width());
                    screen.put_str_at(0, line_y, &clear_str, EFI_BLACK, EFI_BLACK);
                }
            }

            self.last_total_count = total_count;
        }

        if self.completed {
            let displayed_logs = logs_to_show.min(logger::log_count());
            let final_y = logs_start_y + displayed_logs;
            if final_y < screen.height() - 1 {
                let completion_msg = "> System initialized. Press any key...";
                let msg_x = screen.center_x(completion_msg.len());
                screen.put_str_at(
                    msg_x,
                    final_y + 1,
                    completion_msg,
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
            }
        }

        self.last_rendered_count = total_count;
    }

} 