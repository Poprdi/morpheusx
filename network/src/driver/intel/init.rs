//! Intel e1000e initialization sequence.
//!
//! Rust orchestration layer for device initialization.
//! All hardware access is via ASM bindings.
//!
//! # Initialization Sequence (I218/PCH LPT compatible)
//!
//! The sequence below follows the Linux kernel e1000e driver (ich8lan.c)
//! and is CRITICAL for real I218 hardware (ThinkPad T450s, etc.).
//!
//! 1. Disable interrupts
//! 2. Global reset
//! 3. Wait for reset completion
//! 4. Disable interrupts again (reset re-enables)
//! 5. **CRITICAL: Disable ULP (Ultra Low Power) mode**
//!    - I218 may be in ULP after BIOS handoff
//!    - PHY won't respond to MDIC until ULP disabled
//! 6. **CRITICAL: Check PHY accessibility**
//!    - If PHY doesn't respond, toggle LANPHYPC (power cycle)
//! 7. Wake PHY from power down (BMCR.PDOWN)
//! 8. Read MAC address
//! 9. Clear multicast table
//! 10. Setup RX descriptor ring
//! 11. Setup TX descriptor ring
//! 12. Enable RX
//! 13. Enable TX
//! 14. Set link up
//!
//! # Reference
//! - Intel 82579 Datasheet, Section 14 (Initialization)
//! - Linux kernel drivers/net/ethernet/intel/e1000e/ich8lan.c

use crate::asm::drivers::intel::{
    asm_intel_clear_mta, asm_intel_disable_interrupts, asm_intel_enable_rx, asm_intel_enable_tx,
    asm_intel_read_mac, asm_intel_reset, asm_intel_set_link_up, asm_intel_setup_rx_ring,
    asm_intel_setup_tx_ring, phy_read, phy_write,
    // I218/PCH LPT specific functions
    disable_ulp, toggle_lanphypc, phy_is_accessible, acquire_swflag, release_swflag,
};
use crate::dma::DmaRegion;
use crate::mainloop::bare_metal::{serial_print, serial_println, serial_print_decimal};
use crate::types::MacAddress;

use super::regs;
use super::rx::RxRing;
use super::tx::TxRing;

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// E1000e driver configuration.
#[derive(Debug, Clone)]
pub struct E1000eConfig {
    /// Number of RX descriptors.
    pub rx_queue_size: u16,
    /// Number of TX descriptors.
    pub tx_queue_size: u16,
    /// Size of each buffer.
    pub buffer_size: usize,
    /// TSC frequency (ticks per second) for timeouts.
    pub tsc_freq: u64,
    /// DMA region CPU base pointer.
    pub dma_cpu_base: *mut u8,
    /// DMA region bus address.
    pub dma_bus_base: u64,
}

impl E1000eConfig {
    /// Create configuration with default values.
    ///
    /// # Safety
    /// DMA pointers must be valid.
    pub unsafe fn new(dma_cpu_base: *mut u8, dma_bus_base: u64, tsc_freq: u64) -> Self {
        Self {
            rx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            tx_queue_size: regs::DEFAULT_QUEUE_SIZE,
            buffer_size: regs::DEFAULT_BUFFER_SIZE,
            tsc_freq,
            dma_cpu_base,
            dma_bus_base,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

/// Initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E1000eInitError {
    /// Device reset timed out.
    ResetTimeout,
    /// MAC address invalid (all zeros or all ones).
    InvalidMac,
    /// MMIO access failed (device not responding).
    MmioError,
    /// Link did not come up.
    LinkTimeout,
    /// Failed to disable ULP mode (I218 specific).
    UlpDisableFailed,
    /// PHY is not accessible after all recovery attempts.
    PhyNotAccessible,
    /// Failed to acquire hardware semaphore.
    SemaphoreTimeout,
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION RESULT
// ═══════════════════════════════════════════════════════════════════════════

/// Result of successful initialization.
pub struct E1000eInitResult {
    /// MAC address.
    pub mac: MacAddress,
    /// RX ring.
    pub rx_ring: RxRing,
    /// TX ring.
    pub tx_ring: TxRing,
}

// ═══════════════════════════════════════════════════════════════════════════
// INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize the e1000e device.
///
/// # Arguments
/// - `mmio_base`: Device MMIO base address
/// - `config`: Driver configuration
///
/// # Returns
/// Initialization result with MAC and rings, or error.
///
/// # Safety
/// - `mmio_base` must be a valid, mapped MMIO address
/// - DMA region must be properly allocated
pub unsafe fn init_e1000e(
    mmio_base: u64,
    config: &E1000eConfig,
) -> Result<E1000eInitResult, E1000eInitError> {
    serial_println("  [e1000e] Step 1: Disable interrupts");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 1: DISABLE INTERRUPTS
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_disable_interrupts(mmio_base);

    serial_println("  [e1000e] Step 2-3: Global reset");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 2-3: GLOBAL RESET + WAIT
    // ═══════════════════════════════════════════════════════════════════
    let reset_result = asm_intel_reset(mmio_base, config.tsc_freq);
    if reset_result != 0 {
        serial_println("  [e1000e] FAIL: Reset timeout");
        return Err(E1000eInitError::ResetTimeout);
    }

    serial_println("  [e1000e] Step 4: Disable interrupts again");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 4: DISABLE INTERRUPTS AGAIN (reset re-enables them)
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_disable_interrupts(mmio_base);

    serial_println("  [e1000e] Step 5: Disable ULP mode (I218)");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 5: DISABLE ULP MODE (CRITICAL FOR I218)
    //
    // I218-LM/V (ThinkPad T450s, etc.) may be in Ultra Low Power mode
    // after BIOS handoff. The PHY won't respond to MDIC until ULP is
    // disabled. This is THE primary cause of the hang on real hardware!
    //
    // Reference: Linux kernel e1000_disable_ulp_lpt_lp() in ich8lan.c
    // ═══════════════════════════════════════════════════════════════════
    let _ulp_result = disable_ulp(mmio_base, config.tsc_freq);
    // ULP disable may fail on non-I218 hardware, that's OK - continue
    
    serial_println("  [e1000e] Step 6: Ensure PHY accessible");

    // ═══════════════════════════════════════════════════════════════════
    // STEP 6: ENSURE PHY IS ACCESSIBLE
    //
    // After ULP disable, check if PHY responds to MDIC. If not, we need
    // to power cycle the PHY via LANPHYPC toggle.
    //
    // Reference: Linux kernel e1000_init_phy_workarounds_pchlan()
    // ═══════════════════════════════════════════════════════════════════
    if !ensure_phy_accessible(mmio_base, config.tsc_freq) {
        serial_println("  [e1000e] FAIL: PHY not accessible");
        return Err(E1000eInitError::PhyNotAccessible);
    }

    serial_println("  [e1000e] Step 7: Wake PHY from power down");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 7: WAKE PHY FROM POWER DOWN (BMCR.PDOWN)
    //
    // BIOS may have put PHY in power-down mode. We have no ACPI
    // or SMM handler to wake it. Must be done explicitly.
    // ═══════════════════════════════════════════════════════════════════
    wake_phy(mmio_base, config.tsc_freq);

    serial_println("  [e1000e] Step 8: Read MAC address");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 8: READ MAC ADDRESS
    // ═══════════════════════════════════════════════════════════════════
    let mut mac: MacAddress = [0u8; 6];
    let mac_result = asm_intel_read_mac(mmio_base, &mut mac);
    if mac_result != 0 {
        serial_println("  [e1000e] FAIL: Invalid MAC");
        return Err(E1000eInitError::InvalidMac);
    }

    // Validate MAC (not all zeros or all ones)
    if mac == [0, 0, 0, 0, 0, 0] || mac == [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF] {
        serial_println("  [e1000e] FAIL: MAC all 0s or FFs");
        return Err(E1000eInitError::InvalidMac);
    }

    serial_println("  [e1000e] Step 9: Clear multicast table");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 9: CLEAR MULTICAST TABLE
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_clear_mta(mmio_base);

    serial_println("  [e1000e] Step 10: Setup RX ring");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 10: SETUP RX DESCRIPTOR RING
    // ═══════════════════════════════════════════════════════════════════
    let rx_desc_cpu = config.dma_cpu_base.add(DmaRegion::RX_DESC_OFFSET);
    let rx_desc_bus = config.dma_bus_base + DmaRegion::RX_DESC_OFFSET as u64;
    let rx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::RX_BUFFERS_OFFSET);
    let rx_buffer_bus = config.dma_bus_base + DmaRegion::RX_BUFFERS_OFFSET as u64;

    let rx_ring_len_bytes = (config.rx_queue_size as u32) * (regs::DESC_SIZE as u32);

    // Configure hardware RX ring
    asm_intel_setup_rx_ring(mmio_base, rx_desc_bus, rx_ring_len_bytes);

    // Create RX ring structure
    let mut rx_ring = RxRing::new(
        mmio_base,
        rx_desc_cpu,
        rx_desc_bus,
        rx_buffer_cpu,
        rx_buffer_bus,
        config.buffer_size,
        config.rx_queue_size,
    );

    // Initialize all RX descriptors with buffer addresses
    rx_ring.init_descriptors();

    // ═══════════════════════════════════════════════════════════════════
    // STEP 11: SETUP TX DESCRIPTOR RING
    // ═══════════════════════════════════════════════════════════════════
    let tx_desc_cpu = config.dma_cpu_base.add(DmaRegion::TX_DESC_OFFSET);
    let tx_desc_bus = config.dma_bus_base + DmaRegion::TX_DESC_OFFSET as u64;
    let tx_buffer_cpu = config.dma_cpu_base.add(DmaRegion::TX_BUFFERS_OFFSET);
    let tx_buffer_bus = config.dma_bus_base + DmaRegion::TX_BUFFERS_OFFSET as u64;

    let tx_ring_len_bytes = (config.tx_queue_size as u32) * (regs::DESC_SIZE as u32);

    serial_println("  [e1000e] Step 11: Setup TX ring");
    
    // Configure hardware TX ring
    asm_intel_setup_tx_ring(mmio_base, tx_desc_bus, tx_ring_len_bytes);

    // Create TX ring structure
    let mut tx_ring = TxRing::new(
        mmio_base,
        tx_desc_cpu,
        tx_desc_bus,
        tx_buffer_cpu,
        tx_buffer_bus,
        config.buffer_size,
        config.tx_queue_size,
    );

    // Initialize all TX descriptors
    tx_ring.init_descriptors();

    serial_println("  [e1000e] Step 12-14: Enable RX/TX, set link up");
    
    // ═══════════════════════════════════════════════════════════════════
    // STEP 12: ENABLE RX
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_enable_rx(mmio_base);

    // Update RX tail to enable receiving
    rx_ring.update_tail();

    // ═══════════════════════════════════════════════════════════════════
    // STEP 13: ENABLE TX
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_enable_tx(mmio_base);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 14: SET LINK UP
    // ═══════════════════════════════════════════════════════════════════
    asm_intel_set_link_up(mmio_base);

    serial_println("  [e1000e] Init complete!");
    
    // Return success
    Ok(E1000eInitResult {
        mac,
        rx_ring,
        tx_ring,
    })
}

/// Generate a locally-administered MAC address from a seed.
///
/// Used as fallback if EEPROM MAC is invalid.
pub fn generate_fallback_mac(seed: u64) -> MacAddress {
    let mut mac = [0u8; 6];
    let bytes = seed.to_le_bytes();

    // Set locally-administered bit, clear multicast bit
    mac[0] = (bytes[0] & 0xFE) | 0x02;
    mac[1] = bytes[1];
    mac[2] = bytes[2];
    mac[3] = bytes[3];
    mac[4] = bytes[4];
    mac[5] = bytes[5];

    mac
}

// ═══════════════════════════════════════════════════════════════════════════
// I218/PCH LPT PHY ACCESSIBILITY (CRITICAL FOR REAL HARDWARE)
// ═══════════════════════════════════════════════════════════════════════════

/// Ensure PHY is accessible, with recovery via LANPHYPC toggle.
///
/// This is CRITICAL for I218-LM/V (ThinkPad T450s, etc.) - after ULP disable,
/// the PHY may still not respond to MDIC. In that case, we need to power
/// cycle the PHY using the LANPHYPC toggle.
///
/// This function will:
/// 1. Check if PHY responds to MDIC
/// 2. If not, toggle LANPHYPC to power cycle the PHY
/// 3. Re-check PHY accessibility
/// 4. If still not accessible, try forcing SMBus mode
///
/// # Returns
/// `true` if PHY is accessible, `false` if recovery failed.
///
/// # Reference
/// Linux kernel e1000_init_phy_workarounds_pchlan() in ich8lan.c
///
/// # Safety
/// Called during init, MMIO must be valid.
unsafe fn ensure_phy_accessible(mmio_base: u64, tsc_freq: u64) -> bool {
    const MAX_ATTEMPTS: u32 = 3;

    for attempt in 0..MAX_ATTEMPTS {
        serial_print("    PHY check attempt ");
        serial_print_decimal(attempt);
        serial_println("...");
        
        // Check if PHY responds
        if phy_is_accessible(mmio_base, tsc_freq) {
            serial_println("    PHY accessible!");
            return true;
        }

        serial_println("    PHY not responding, trying recovery...");
        
        // PHY not accessible - try recovery based on attempt number
        match attempt {
            0 => {
                // First attempt: just wait a bit longer after ULP disable
                // Some I218 variants need extra time
                serial_println("    Recovery: waiting 50ms...");
                let start = crate::asm::core::tsc::read_tsc();
                let delay = tsc_freq / 20; // 50ms
                while crate::asm::core::tsc::read_tsc().wrapping_sub(start) < delay {
                    core::hint::spin_loop();
                }
            }
            1 => {
                // Second attempt: toggle LANPHYPC to power cycle PHY
                serial_println("    Recovery: toggling LANPHYPC...");
                let _ = toggle_lanphypc(mmio_base, tsc_freq);
            }
            2 => {
                // Third attempt: force SMBus mode and toggle again
                serial_println("    Recovery: SMBus mode + LANPHYPC...");
                crate::asm::drivers::intel::force_smbus_mode(mmio_base);
                let _ = toggle_lanphypc(mmio_base, tsc_freq);
                crate::asm::drivers::intel::clear_smbus_mode(mmio_base);
            }
            _ => {}
        }
    }

    serial_println("    Final PHY check...");
    // Final check after all recovery attempts
    phy_is_accessible(mmio_base, tsc_freq)
}

// ═══════════════════════════════════════════════════════════════════════════
// POWER MANAGEMENT HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Wake PHY from power-down mode, reset it, and restart auto-negotiation.
///
/// CRITICAL for post-ExitBootServices operation on real hardware!
///
/// BIOS may have enabled PHY power management (BMCR.PDOWN). In a normal
/// OS environment, ACPI or SMM handlers would wake the PHY. Post-EBS,
/// we are on our own - must explicitly:
/// 1. Clear PDOWN to wake PHY
/// 2. Wait for PHY to stabilize (100ms - PLL and analog circuitry)
/// 3. Issue PHY reset (BMCR.RESET)
/// 4. Wait for reset to complete
/// 5. Restart auto-negotiation
///
/// # Arguments
/// - `mmio_base`: Device MMIO base address
/// - `tsc_freq`: TSC frequency for timeout calculation
///
/// # Safety
/// Called during init, MMIO must be valid.
unsafe fn wake_phy(mmio_base: u64, tsc_freq: u64) {
    // ═══════════════════════════════════════════════════════════════════
    // STEP 1: Wake PHY from power-down mode
    // ═══════════════════════════════════════════════════════════════════
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        if bmcr & regs::BMCR_PDOWN != 0 {
            // Clear PDOWN bit to wake PHY
            let new_bmcr = bmcr & !regs::BMCR_PDOWN;
            let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
        }
    }

    // Also clear ISOLATE bit which can prevent operation
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        if bmcr & regs::BMCR_ISOLATE != 0 {
            let new_bmcr = bmcr & !regs::BMCR_ISOLATE;
            let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // STEP 2: Wait for PHY to wake (100ms)
    //
    // Intel datasheet specifies PHY needs 50-100ms after PDOWN clear
    // for PLL lock and analog circuitry stabilization. QEMU doesn't
    // need this, but real hardware absolutely does.
    // ═══════════════════════════════════════════════════════════════════
    let start = crate::asm::core::tsc::read_tsc();
    let delay_ticks = tsc_freq / 10; // 100ms (not 1ms!)
    while crate::asm::core::tsc::read_tsc().wrapping_sub(start) < delay_ticks {
        core::hint::spin_loop();
    }

    // ═══════════════════════════════════════════════════════════════════
    // STEP 3: Issue PHY reset (BMCR.RESET)
    //
    // Real hardware may be in an inconsistent state after BIOS handoff.
    // PHY reset establishes a clean baseline for operation.
    // ═══════════════════════════════════════════════════════════════════
    let _ = phy_write(mmio_base, regs::PHY_BMCR, regs::BMCR_RESET, tsc_freq);

    // ═══════════════════════════════════════════════════════════════════
    // STEP 4: Wait for PHY reset to complete (poll BMCR.RESET bit)
    //
    // The PHY clears the RESET bit when reset is complete.
    // Timeout after 500ms (generous for real hardware).
    // ═══════════════════════════════════════════════════════════════════
    let reset_start = crate::asm::core::tsc::read_tsc();
    let reset_timeout = tsc_freq / 2; // 500ms timeout
    loop {
        if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
            if bmcr & regs::BMCR_RESET == 0 {
                // Reset complete
                break;
            }
        }
        if crate::asm::core::tsc::read_tsc().wrapping_sub(reset_start) >= reset_timeout {
            // Timeout - continue anyway, some PHYs may not clear the bit
            break;
        }
        core::hint::spin_loop();
    }

    // Small delay after reset before continuing (10ms)
    let post_reset_start = crate::asm::core::tsc::read_tsc();
    let post_reset_delay = tsc_freq / 100; // 10ms
    while crate::asm::core::tsc::read_tsc().wrapping_sub(post_reset_start) < post_reset_delay {
        core::hint::spin_loop();
    }

    // ═══════════════════════════════════════════════════════════════════
    // STEP 5: Restart auto-negotiation
    //
    // After reset, the PHY needs to re-negotiate link parameters with
    // the link partner. Without this, link may never come up.
    // ═══════════════════════════════════════════════════════════════════
    if let Some(bmcr) = phy_read(mmio_base, regs::PHY_BMCR, tsc_freq) {
        let new_bmcr = bmcr | regs::BMCR_ANENABLE | regs::BMCR_ANRESTART;
        let _ = phy_write(mmio_base, regs::PHY_BMCR, new_bmcr, tsc_freq);
    }

    // Small delay after starting autoneg (10ms)
    let autoneg_start = crate::asm::core::tsc::read_tsc();
    let autoneg_delay = tsc_freq / 100; // 10ms
    while crate::asm::core::tsc::read_tsc().wrapping_sub(autoneg_start) < autoneg_delay {
        core::hint::spin_loop();
    }
}
