use alloc::boxed::Box;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW};
use morpheus_core::logger;
use morpheus_core::net::{
    NetworkInit, NetworkInitResult, InitConfig, NetworkStatus,
    error_log_pop, error_log_clear, ErrorLogEntry,
};

/// Result of network initialization for bootstrap phase
pub enum NetworkBootResult {
    /// Network initialized successfully
    Success(NetworkInitResult),
    /// Network initialization failed
    Failed,
    /// Network initialization skipped (e.g., no config)
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
    /// On success: logs "Network initialized" with IP address.
    /// On failure: dumps all error logs from the ring buffer.
    ///
    /// # Arguments
    /// * `screen` - Screen for rendering status
    /// * `x`, `y` - Position for boot sequence display
    /// * `get_time_ms` - Function returning current time in milliseconds
    ///
    /// # Returns
    /// `NetworkBootResult` indicating success (with client), failure, or skipped.
    pub fn init_network(
        &mut self,
        screen: &mut Screen,
        x: usize,
        y: usize,
        get_time_ms: fn() -> u64,
    ) -> NetworkBootResult {
        // Clear any previous network errors
        error_log_clear();

        // Log that we're starting network init
        logger::log("Initializing network stack...");
        self.render(screen, x, y);

        // Use QEMU Q35 config by default
        let config = InitConfig::for_qemu();

        match NetworkInit::initialize(&config, get_time_ms) {
            Ok(result) => {
                // Success! Log the IP address
                let ip = result.status.ip_address;
                let mac = result.status.mac_address;
                let time_ms = result.status.init_time_ms;

                // Format success message with IP
                let mut msg_buf = [0u8; 64];
                let msg = format_net_success(&mut msg_buf, &ip, time_ms);
                logger::log(msg);
                self.render(screen, x, y);

                NetworkBootResult::Success(result)
            }
            Err(e) => {
                // Failure - log the error type
                logger::log("Network initialization FAILED");
                self.render(screen, x, y);

                // Display all error logs from ring buffer
                self.display_network_errors(screen, x, y);

                NetworkBootResult::Failed
            }
        }
    }

    /// Display network error logs below the boot sequence.
    fn display_network_errors(&self, screen: &mut Screen, x: usize, y: usize) {
        // Find position below current logs
        let logs_shown = logger::log_count().min(20);
        let error_start_y = y + logs_shown + 2;

        // Header
        if error_start_y < screen.height() {
            screen.put_str_at(x, error_start_y, "Network Error Details:", EFI_RED, EFI_BLACK);
        }

        // Dump error ring buffer
        let mut line = 1;
        let max_errors = 10; // Don't flood the screen
        
        while let Some(entry) = error_log_pop() {
            if line > max_errors {
                break;
            }

            let err_y = error_start_y + line;
            if err_y >= screen.height() - 1 {
                break;
            }

            // Format: "[STAGE] message"
            let mut format_buf = [0u8; 120];
            let len = entry.format(&mut format_buf);
            let formatted = core::str::from_utf8(&format_buf[..len]).unwrap_or("?");

            // Color based on error vs debug
            let color = if entry.is_error { EFI_RED } else { EFI_YELLOW };
            
            // Clear line and display
            screen.put_str_at(x, err_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
            screen.put_str_at(x + 2, err_y, formatted, color, EFI_BLACK);

            line += 1;
        }
    }
}

/// Format network success message into buffer, returns &'static str.
/// 
/// Message format: "Network ready: IP x.x.x.x (XXXms)"
fn format_net_success(buf: &mut [u8; 64], ip: &[u8; 4], time_ms: u64) -> &'static str {
    use core::fmt::Write;
    
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    
    impl<'a> Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let remaining = self.buf.len() - self.pos;
            let to_copy = bytes.len().min(remaining);
            self.buf[self.pos..self.pos + to_copy].copy_from_slice(&bytes[..to_copy]);
            self.pos += to_copy;
            Ok(())
        }
    }
    
    let mut writer = BufWriter { buf: buf, pos: 0 };
    let _ = write!(
        writer,
        "Network ready: IP {}.{}.{}.{} ({}ms)",
        ip[0], ip[1], ip[2], ip[3], time_ms
    );
    
    // Leak the formatted string so we can return &'static str
    // This is fine because we only call this once during boot
    let len = writer.pos;
    let static_buf: &'static mut [u8; 64] = Box::leak(Box::new(*buf));
    core::str::from_utf8(&static_buf[..len]).unwrap_or("Network ready")
}
