//! Display and UI functions for download commit flow.

extern crate alloc;

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

/// Display preparation phase header.
pub fn display_preparation_header(screen: &mut Screen) -> usize {
    screen.clear();
    screen.put_str_at(
        5,
        2,
        "=== Preparing Download Environment ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    4 // Return starting log line
}

/// Display final pre-exit messages.
pub fn display_final_warnings(screen: &mut Screen, log_y: &mut usize, bs: &crate::BootServices) {
    screen.put_str_at(
        5,
        *log_y,
        "=== EXITING BOOT SERVICES ===",
        EFI_RED,
        EFI_BLACK,
    );
    *log_y += 1;
    screen.put_str_at(5, *log_y, "After this point:", EFI_YELLOW, EFI_BLACK);
    *log_y += 1;
    screen.put_str_at(
        7,
        *log_y,
        "- Screen output will stop",
        EFI_YELLOW,
        EFI_BLACK,
    );
    *log_y += 1;
    screen.put_str_at(
        7,
        *log_y,
        "- Progress via serial console only",
        EFI_YELLOW,
        EFI_BLACK,
    );
    *log_y += 1;
    screen.put_str_at(
        7,
        *log_y,
        "- System will reboot when done",
        EFI_YELLOW,
        EFI_BLACK,
    );
    *log_y += 2;

    screen.put_str_at(
        5,
        *log_y,
        "Exiting boot services NOW...",
        EFI_RED,
        EFI_BLACK,
    );
    *log_y += 1;
    screen.put_str_at(
        7,
        *log_y,
        "(screen will freeze - progress on serial)",
        EFI_YELLOW,
        EFI_BLACK,
    );

    // Brief pause for user to see message
    let _ = (bs.stall)(500_000); // 500ms
}
