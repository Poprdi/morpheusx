//! Network connectivity check for distro downloader
//!
//! Assumes network has already been initialized in bootstrap phase.
//! Provides connectivity verification before attempting downloads.

use crate::screen::Screen;
use crate::uefi::EFI_LIGHTGREEN;
use crate::uefi::EFI_YELLOW;
use crate::uefi::EFI_RED;
use crate::uefi::EFI_DARKGRAY;
use crate::uefi::EFI_BLACK;
use dma_pool::DmaPool;

/// Check network connectivity
///
/// Verifies that network stack was properly initialized during bootstrap:
/// 1. DMA pool is initialized and valid
/// 2. Network stack has seen traffic (RX packets > 0 indicates DHCP worked)
/// 3. DMA pool is in valid address range for VirtIO (<4GB)
///
/// # Arguments
/// * `screen` - Display screen for user feedback
///
/// # Returns
/// - `Ok(())` - Network appears to be ready
/// - `Err(msg)` - Error message if connectivity check fails
pub fn check_network_connectivity(screen: &mut Screen) -> Result<(), &'static str> {
    screen.clear();
    screen.put_str_at(5, 2, "=== Network Connectivity Check ===", EFI_LIGHTGREEN, EFI_BLACK);
    
    let mut log_y = 4;

    // Check 1: Verify DMA pool is initialized
    screen.put_str_at(5, log_y, "Checking DMA pool...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    let pool_base = DmaPool::base_address();
    let pool_size = DmaPool::total_size();
    
    if pool_size == 0 {
        screen.put_str_at(7, log_y, "DMA pool not initialized!", EFI_RED, EFI_BLACK);
        log_y += 2;
        screen.put_str_at(5, log_y, "Network stack requires DMA pool", EFI_DARKGRAY, EFI_BLACK);
        log_y += 1;
        screen.put_str_at(5, log_y, "Should be initialized during bootstrap", EFI_DARKGRAY, EFI_BLACK);
        return Err("DMA pool not initialized - network not set up");
    }
    
    screen.put_str_at(7, log_y, &format!(
        "Base: {:#x}, Size: {} KB",
        pool_base, pool_size / 1024
    ), EFI_DARKGRAY, EFI_BLACK);
    log_y += 1;
    
    // Check if DMA pool is in valid range (<4GB for VirtIO)
    let pool_end = pool_base + pool_size;
    let pool_valid = pool_base < 0x1_0000_0000 && pool_end <= 0x1_0000_0000;
    
    if !pool_valid {
        screen.put_str_at(7, log_y, "WARNING: DMA pool >4GB", EFI_RED, EFI_BLACK);
        log_y += 1;
        screen.put_str_at(7, log_y, "VirtIO may not work correctly", EFI_DARKGRAY, EFI_BLACK);
        // Don't fail completely, but warn
    } else {
        screen.put_str_at(7, log_y, "DMA range: <4GB (OK)", EFI_LIGHTGREEN, EFI_BLACK);
    }
    log_y += 2;
    
    // Check 2: Verify network stack has seen traffic
    screen.put_str_at(5, log_y, "Checking network stack...", EFI_YELLOW, EFI_BLACK);
    log_y += 1;
    
    let rx_count = morpheus_network::stack::rx_packet_count();
    let tx_count = morpheus_network::stack::tx_packet_count();
    
    screen.put_str_at(7, log_y, &format!(
        "RX: {} packets, TX: {} packets",
        rx_count, tx_count
    ), EFI_DARKGRAY, EFI_BLACK);
    log_y += 1;
    
    // If we've received packets, it means:
    // - VirtIO device is working
    // - DHCP likely completed (we got DHCP offer/ack)
    // - Network stack is functional
    if rx_count == 0 {
        screen.put_str_at(7, log_y, "No RX packets - DHCP may have failed", EFI_RED, EFI_BLACK);
        log_y += 2;
        screen.put_str_at(5, log_y, "Network stack appears inactive", EFI_DARKGRAY, EFI_BLACK);
        log_y += 1;
        screen.put_str_at(5, log_y, "Bootstrap should complete DHCP first", EFI_DARKGRAY, EFI_BLACK);
        return Err("No network traffic detected - DHCP not completed");
    }
    
    screen.put_str_at(7, log_y, "Network traffic detected (OK)", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 2;
    
    // Success!
    screen.put_str_at(5, log_y, "Network connectivity verified!", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 2;
    
    screen.put_str_at(5, log_y, "Press any key to continue...", EFI_DARKGRAY, EFI_BLACK);
    
    // Small delay so user can see the check results
    fn get_time_ms() -> u64 {
        let tsc = unsafe { morpheus_network::read_tsc() };
        tsc / 2_000_000
    }
    let pause_start = get_time_ms();
    while get_time_ms() - pause_start < 1500 {}
    
    Ok(())
}
