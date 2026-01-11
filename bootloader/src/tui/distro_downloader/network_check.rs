//! Network connectivity verification for distro downloader
//!
//! ARCHITECTURE CHANGE: Network initialization now happens post-ExitBootServices.
//! This module is DEPRECATED - network check is no longer done during UEFI phase.
//!
//! The new flow is:
//! 1. User browses catalog (no network needed - static data)
//! 2. User selects ISO and confirms download
//! 3. ExitBootServices is called
//! 4. Bare-metal network stack initializes (VirtIO + smoltcp)  
//! 5. Download proceeds in bare-metal mode
//!
//! This check function now always succeeds since network isn't initialized yet.

extern crate alloc;

use crate::tui::renderer::{Screen, EFI_BLACK, EFI_LIGHTGREEN, EFI_YELLOW};

/// Check network connectivity (DEPRECATED - always succeeds)
///
/// Previously verified network was initialized during bootstrap.
/// Now network init is deferred to download time (post-ExitBootServices).
/// This function is kept for API compatibility but always returns Ok.
///
/// # Arguments
/// * `screen` - Display screen for user feedback
///
/// # Returns
/// Always returns `Ok(())` - network will be initialized post-EBS
#[deprecated(note = "Network init moved to post-EBS. Remove this check.")]
pub fn check_network_connectivity(screen: &mut Screen) -> Result<(), &'static str> {
    screen.clear();
    screen.put_str_at(5, 2, "=== Network Status ===", EFI_LIGHTGREEN, EFI_BLACK);

    let mut log_y = 4;

    // Inform user about deferred network init
    screen.put_str_at(
        5,
        log_y,
        "Network initialization is deferred.",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 2;

    screen.put_str_at(5, log_y, "When download starts:", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "1. UEFI Boot Services will exit",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "2. VirtIO network driver initializes",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "3. DHCP acquires IP address",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 1;
    screen.put_str_at(
        7,
        log_y,
        "4. Download proceeds in bare-metal mode",
        EFI_YELLOW,
        EFI_BLACK,
    );
    log_y += 2;

    screen.put_str_at(5, log_y, "Ready to proceed!", EFI_LIGHTGREEN, EFI_BLACK);

    // Brief pause so user can see message
    for _ in 0..50_000_000 {
        core::hint::spin_loop();
    }

    Ok(())
}
