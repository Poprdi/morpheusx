//! Display and UI functions for download commit flow.

extern crate alloc;

use crate::tui::logo::LOGO_LINES_RAW;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_LIGHTGREEN, EFI_RED, EFI_YELLOW};

/// Download commit configuration.
pub struct DownloadCommitConfig {
    /// URL to download from
    pub iso_url: alloc::string::String,
    /// Expected size in bytes (for progress)
    pub iso_size: u64,
    /// Name of the distro (for display)
    pub distro_name: alloc::string::String,
}

/// Display countdown before committing to download.
pub fn display_commit_countdown(
    screen: &mut Screen,
    config: &DownloadCommitConfig,
    bs: &crate::BootServices,
) {
    screen.clear();

    screen.put_str_at(
        5,
        2,
        "=== Download Confirmation ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );

    screen.put_str_at(5, 4, "About to download:", EFI_YELLOW, EFI_BLACK);
    screen.put_str_at(7, 5, &config.distro_name, EFI_CYAN, EFI_BLACK);
    screen.put_str_at(
        7,
        6,
        &alloc::format!("Size: {} MB", config.iso_size / (1024 * 1024)),
        EFI_CYAN,
        EFI_BLACK,
    );

    screen.put_str_at(
        5,
        8,
        "WARNING: This will exit UEFI boot services!",
        EFI_RED,
        EFI_BLACK,
    );
    screen.put_str_at(
        5,
        9,
        "The system cannot be interrupted during download.",
        EFI_RED,
        EFI_BLACK,
    );

    // Countdown with UEFI Stall (1 second = 1,000,000 microseconds)
    screen.put_str_at(5, 11, "Starting in 3...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);

    screen.put_str_at(5, 11, "Starting in 2...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);

    screen.put_str_at(5, 11, "Starting in 1...", EFI_YELLOW, EFI_BLACK);
    let _ = (bs.stall)(1_000_000);
}

/// Debug log buffer for conditional display.
pub struct DebugLog {
    lines: [([u8; 80], u8); 32], // 32 lines, 80 chars each, with color
    count: usize,
}

impl DebugLog {
    /// Create empty debug log.
    pub const fn new() -> Self {
        Self {
            lines: [([0u8; 80], 0); 32],
            count: 0,
        }
    }

    /// Add a line to the log.
    pub fn add(&mut self, msg: &str, color: u8) {
        if self.count < 32 {
            let bytes = msg.as_bytes();
            let len = bytes.len().min(80);
            self.lines[self.count].0[..len].copy_from_slice(&bytes[..len]);
            self.lines[self.count].1 = color;
            self.count += 1;
        }
    }

    /// Display the debug log on screen (only called on error).
    pub fn display(&self, screen: &mut Screen) {
        screen.clear();
        screen.put_str_at(
            5,
            1,
            "=== INITIALIZATION ERROR - DEBUG LOG ===",
            EFI_RED,
            EFI_BLACK,
        );

        for i in 0..self.count {
            let line = &self.lines[i];
            // Find actual string length
            let len = line.0.iter().position(|&c| c == 0).unwrap_or(80);
            if len > 0 {
                // Safe conversion - we only stored ASCII
                if let Ok(s) = core::str::from_utf8(&line.0[..len]) {
                    let color = match line.1 {
                        1 => EFI_YELLOW,
                        2 => EFI_LIGHTGREEN,
                        3 => EFI_RED,
                        4 => EFI_CYAN,
                        _ => EFI_YELLOW,
                    };
                    screen.put_str_at(5, 3 + i, s, color, EFI_BLACK);
                }
            }
        }
    }
}

/// Color constants for debug log.
pub const LOG_YELLOW: u8 = 1;
pub const LOG_GREEN: u8 = 2;
pub const LOG_RED: u8 = 3;
pub const LOG_CYAN: u8 = 4;

/// Display clean ASCII art and final message before download (success path).
pub fn display_download_start(screen: &mut Screen, bs: &crate::BootServices) {
    screen.clear();

    // Use the main MorpheusX logo
    let start_y = 6;
    for (i, line) in LOGO_LINES_RAW.iter().enumerate() {
        screen.put_str_at(43, start_y + i, line, EFI_LIGHTGREEN, EFI_BLACK);
    }

    // Message box below logo
    let msg_y = start_y + LOGO_LINES_RAW.len() + 2;

    screen.put_str_at(
        43,
        msg_y,
        "╔══════════════════════════════════════════════════════════════════╗",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 1,
        "║                                                                  ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 2,
        "║                   DOWNLOAD STARTING NOW                          ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 3,
        "║                                                                  ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 4,
        "║              System will reboot when finished                    ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 5,
        "║                                                                  ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 6,
        "║                     ⚠  DO NOT INTERRUPT  ⚠                         ║",
        EFI_RED,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 7,
        "║                                                                  ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 8,
        "║          Download may take a while depending on size!            ║",
        EFI_CYAN,
        EFI_BLACK,
    );
     screen.put_str_at(
        43,
        msg_y + 9,
        "║  No its not stuck we just dont have a post EBS display stack yet ║",
        EFI_CYAN,
        EFI_BLACK,
    );
    screen.put_str_at(
        43,
        msg_y + 10,
        "╚══════════════════════════════════════════════════════════════════╝",
        EFI_CYAN,
        EFI_BLACK,
    );

    // Brief pause so user can see the message
    let _ = (bs.stall)(1_430_000); // 1.5 seconds
}

/// Display error screen with debug log and halt message.
pub fn display_error_and_halt(
    screen: &mut Screen,
    debug_log: &DebugLog,
    error_msg: &str,
    bs: &crate::BootServices,
) -> ! {
    debug_log.display(screen);

    let error_y = 3 + debug_log.count + 2;
    screen.put_str_at(5, error_y, "FATAL ERROR:", EFI_RED, EFI_BLACK);
    screen.put_str_at(7, error_y + 1, error_msg, EFI_RED, EFI_BLACK);
    screen.put_str_at(
        5,
        error_y + 3,
        "System halted. Please reboot manually.",
        EFI_YELLOW,
        EFI_BLACK,
    );

    // Give time to read
    let _ = (bs.stall)(5_000_000);

    loop {
        core::hint::spin_loop();
    }
}
